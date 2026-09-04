//! Pairing cryptography and persistent client identity.

use std::fs;
use std::path::{Path, PathBuf};

use aes::Aes128;
use aes::cipher::{Block, BlockDecrypt, BlockEncrypt, KeyInit};
use rand::RngCore;
use rand::rngs::OsRng;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_RSA_SHA256};
use rsa::pkcs1v15::{Signature, SigningKey, VerifyingKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, LineEnding};
use rsa::signature::{SignatureEncoding, Signer, Verifier};
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::{DecodePem, Encode};

use crate::client::ControlError;

const CERTIFICATE_FILE: &str = "client.pem";
const PRIVATE_KEY_FILE: &str = "key.pem";
const UNIQUE_ID_FILE: &str = "unique_id";

/// Long-lived Moonlight client identity used for pairing and mutual TLS.
pub(crate) struct ClientIdentity {
    certificate_pem: String,
    private_key_pem: String,
    private_key: RsaPrivateKey,
    certificate_signature: Vec<u8>,
    unique_id: String,
}

impl ClientIdentity {
    /// Load the identity from `directory`, creating it when no files exist.
    pub(crate) fn load_or_create(directory: &Path) -> Result<Self, ControlError> {
        let certificate_path = directory.join(CERTIFICATE_FILE);
        let private_key_path = directory.join(PRIVATE_KEY_FILE);
        let unique_id_path = directory.join(UNIQUE_ID_FILE);
        let existing = [
            certificate_path.exists(),
            private_key_path.exists(),
            unique_id_path.exists(),
        ];

        if existing.iter().all(|value| *value) {
            return Self::from_pem(
                fs::read_to_string(certificate_path)?,
                fs::read_to_string(private_key_path)?,
                fs::read_to_string(unique_id_path)?.trim().to_owned(),
            );
        }
        if existing.iter().any(|value| *value) {
            return Err(ControlError::Identity(
                "client identity is incomplete; refusing to overwrite it".to_owned(),
            ));
        }

        let identity = Self::generate()?;
        fs::create_dir_all(directory)?;
        fs::write(certificate_path, identity.certificate_pem.as_bytes())?;
        fs::write(private_key_path, identity.private_key_pem.as_bytes())?;
        fs::write(unique_id_path, format!("{}\n", identity.unique_id))?;
        Ok(identity)
    }

    /// Generate an in-memory identity for tests and transient clients.
    pub(crate) fn generate() -> Result<Self, ControlError> {
        let mut random = OsRng;
        let private_key = RsaPrivateKey::new(&mut random, 2048)
            .map_err(|error| ControlError::Identity(error.to_string()))?;
        let private_key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|error| ControlError::Identity(error.to_string()))?
            .to_string();
        let key_pair = KeyPair::from_pkcs8_pem_and_sign_algo(&private_key_pem, &PKCS_RSA_SHA256)
            .map_err(|error| ControlError::Identity(error.to_string()))?;

        let mut parameters = CertificateParams::new(Vec::<String>::new())
            .map_err(|error| ControlError::Identity(error.to_string()))?;
        let mut distinguished_name = DistinguishedName::new();
        distinguished_name.push(DnType::CommonName, "NVIDIA GameStream Client");
        parameters.distinguished_name = distinguished_name;
        parameters.serial_number = Some(0_u64.into());
        let certificate = parameters
            .self_signed(&key_pair)
            .map_err(|error| ControlError::Identity(error.to_string()))?;

        let mut unique_id_bytes = [0_u8; 8];
        random.fill_bytes(&mut unique_id_bytes);
        let unique_id = hex::encode(unique_id_bytes);
        Self::from_pem(certificate.pem(), private_key_pem, unique_id)
    }

    fn from_pem(
        certificate_pem: String,
        private_key_pem: String,
        unique_id: String,
    ) -> Result<Self, ControlError> {
        if unique_id.is_empty() || !unique_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ControlError::Identity(
                "unique ID must be a non-empty hexadecimal string".to_owned(),
            ));
        }
        let private_key = RsaPrivateKey::from_pkcs8_pem(&private_key_pem)
            .map_err(|error| ControlError::Identity(error.to_string()))?;
        let certificate = Certificate::from_pem(certificate_pem.as_bytes())
            .map_err(|error| ControlError::Identity(error.to_string()))?;
        let certificate_public_key = public_key(&certificate)?;
        if certificate_public_key != RsaPublicKey::from(&private_key) {
            return Err(ControlError::Identity(
                "client certificate and private key do not match".to_owned(),
            ));
        }
        let certificate_signature = certificate
            .signature
            .as_bytes()
            .ok_or_else(|| ControlError::Identity("invalid certificate signature bits".to_owned()))?
            .to_vec();

        Ok(Self {
            certificate_pem,
            private_key_pem,
            private_key,
            certificate_signature,
            unique_id,
        })
    }

    pub(crate) fn unique_id(&self) -> &str {
        &self.unique_id
    }

    pub(crate) fn certificate_pem(&self) -> &str {
        &self.certificate_pem
    }

    pub(crate) fn reqwest_identity(&self) -> Result<reqwest::Identity, ControlError> {
        let combined = format!("{}{}", self.certificate_pem, self.private_key_pem);
        reqwest::Identity::from_pem(combined.as_bytes())
            .map_err(|error| ControlError::Identity(error.to_string()))
    }

    pub(crate) fn certificate_signature(&self) -> &[u8] {
        &self.certificate_signature
    }

    pub(crate) fn sign(&self, message: &[u8]) -> Vec<u8> {
        let signing_key = SigningKey::<Sha256>::new(self.private_key.clone());
        signing_key.sign(message).to_vec()
    }
}

pub(crate) fn default_identity_directory() -> Result<PathBuf, ControlError> {
    if let Some(path) = std::env::var_os("MOONLIGHT_SCARLET_CONFIG_DIR") {
        return Ok(PathBuf::from(path));
    }

    #[cfg(target_os = "scarlet")]
    let directory = PathBuf::from("/etc/moonlight-scarlet");

    #[cfg(target_os = "macos")]
    let directory = {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            ControlError::Identity("HOME is not set; configure MOONLIGHT_SCARLET_CONFIG_DIR".into())
        })?;
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Moonlight Scarlet")
    };

    #[cfg(not(any(target_os = "scarlet", target_os = "macos")))]
    let directory = {
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
            PathBuf::from(path).join("moonlight-scarlet")
        } else {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                ControlError::Identity(
                    "HOME is not set; configure MOONLIGHT_SCARLET_CONFIG_DIR".into(),
                )
            })?;
            PathBuf::from(home)
                .join(".config")
                .join("moonlight-scarlet")
        }
    };

    Ok(directory)
}

pub(crate) fn random_bytes<const SIZE: usize>() -> [u8; SIZE] {
    let mut bytes = [0_u8; SIZE];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

pub(crate) fn hash_for_server(input: &[u8], server_major_version: u32) -> Vec<u8> {
    if server_major_version >= 7 {
        Sha256::digest(input).to_vec()
    } else {
        Sha1::digest(input).to_vec()
    }
}

pub(crate) fn aes_encrypt_zero_padded(
    plaintext: &[u8],
    key: &[u8; 16],
) -> Result<Vec<u8>, ControlError> {
    if plaintext.is_empty() {
        return Ok(Vec::new());
    }
    let padded_length = plaintext
        .len()
        .checked_add(15)
        .map(|length| length & !15)
        .ok_or_else(|| ControlError::Crypto("AES input is too large".to_owned()))?;
    let mut output = vec![0_u8; padded_length];
    output[..plaintext.len()].copy_from_slice(plaintext);
    let cipher =
        Aes128::new_from_slice(key).map_err(|error| ControlError::Crypto(error.to_string()))?;
    for chunk in output.chunks_exact_mut(16) {
        cipher.encrypt_block(Block::<Aes128>::from_mut_slice(chunk));
    }
    Ok(output)
}

pub(crate) fn aes_decrypt(ciphertext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, ControlError> {
    if !ciphertext.len().is_multiple_of(16) {
        return Err(ControlError::Crypto(
            "AES ciphertext length is not a multiple of 16".to_owned(),
        ));
    }
    let cipher =
        Aes128::new_from_slice(key).map_err(|error| ControlError::Crypto(error.to_string()))?;
    let mut output = ciphertext.to_vec();
    for chunk in output.chunks_exact_mut(16) {
        cipher.decrypt_block(Block::<Aes128>::from_mut_slice(chunk));
    }
    Ok(output)
}

pub(crate) fn certificate_signature(certificate_pem: &[u8]) -> Result<Vec<u8>, ControlError> {
    let certificate = Certificate::from_pem(certificate_pem)
        .map_err(|error| ControlError::Crypto(error.to_string()))?;
    certificate
        .signature
        .as_bytes()
        .map(ToOwned::to_owned)
        .ok_or_else(|| ControlError::Crypto("invalid certificate signature bits".to_owned()))
}

pub(crate) fn verify_signature(
    certificate_pem: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), ControlError> {
    let certificate = Certificate::from_pem(certificate_pem)
        .map_err(|error| ControlError::Crypto(error.to_string()))?;
    let verifying_key = VerifyingKey::<Sha256>::new(public_key(&certificate)?);
    let signature =
        Signature::try_from(signature).map_err(|error| ControlError::Crypto(error.to_string()))?;
    verifying_key
        .verify(message, &signature)
        .map_err(|_| ControlError::Pairing("server signature verification failed".to_owned()))
}

fn public_key(certificate: &Certificate) -> Result<RsaPublicKey, ControlError> {
    let encoded = certificate
        .tbs_certificate
        .subject_public_key_info
        .to_der()
        .map_err(|error| ControlError::Crypto(error.to_string()))?;
    RsaPublicKey::from_public_key_der(&encoded)
        .map_err(|error| ControlError::Crypto(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{
        ClientIdentity, aes_decrypt, aes_encrypt_zero_padded, certificate_signature,
        verify_signature,
    };
    use tempfile::tempdir;

    #[test]
    fn identity_round_trips_through_disk() {
        let directory = tempdir().expect("temporary directory");
        let first = ClientIdentity::load_or_create(directory.path()).expect("generate identity");
        let second = ClientIdentity::load_or_create(directory.path()).expect("load identity");

        assert_eq!(first.unique_id(), second.unique_id());
        assert_eq!(first.certificate_pem(), second.certificate_pem());
    }

    #[test]
    fn generated_identity_signatures_verify() {
        let identity = ClientIdentity::generate().expect("generate identity");
        let message = b"moonlight pairing secret";
        let signature = identity.sign(message);

        verify_signature(identity.certificate_pem().as_bytes(), message, &signature)
            .expect("verify signature");
        assert_eq!(
            certificate_signature(identity.certificate_pem().as_bytes())
                .expect("certificate signature"),
            identity.certificate_signature()
        );
    }

    #[test]
    fn aes_ecb_round_trip_preserves_payload() {
        let key = [0x31; 16];
        let plaintext = b"a non-block-sized pairing value";
        let encrypted = aes_encrypt_zero_padded(plaintext, &key).expect("encrypt");
        let decrypted = aes_decrypt(&encrypted, &key).expect("decrypt");

        assert_eq!(&decrypted[..plaintext.len()], plaintext);
        assert!(decrypted[plaintext.len()..].iter().all(|byte| *byte == 0));
    }
}
