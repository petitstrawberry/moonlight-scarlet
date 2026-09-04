use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let source_dir = manifest_dir.join("../third_party/moonlight-common-c");
    let mbedtls_dir = tstrans_mbedtls_src::source_dir();
    let mbedtls_config = manifest_dir.join("platform/mbedtls_config.h");
    let common_bridge = manifest_dir.join("platform/common_bridge.c");
    let scarlet_platform = manifest_dir.join("platform/scarlet");
    let is_scarlet = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("scarlet");

    emit_rerun_directives(&source_dir);
    emit_rerun_directives(&scarlet_platform);
    println!("cargo:rerun-if-changed={}", mbedtls_config.display());
    println!("cargo:rerun-if-changed={}", common_bridge.display());

    build_mbedcrypto(
        &mbedtls_dir,
        &mbedtls_config,
        is_scarlet
            .then(|| scarlet_platform.join("include"))
            .as_deref(),
    );

    let mut build = cc::Build::new();
    if is_scarlet {
        build.include(scarlet_platform.join("include"));
    }
    build
        .include(source_dir.join("src"))
        .include(source_dir.join("enet/include"))
        .include(source_dir.join("nanors"))
        .include(source_dir.join("nanors/deps"))
        .include(source_dir.join("nanors/deps/obl"))
        .include(mbedtls_dir.join("include"))
        .include(mbedtls_dir.join("library"))
        .define("USE_MBEDTLS", None)
        .define("HAS_SOCKLEN_T", None)
        .define("NDEBUG", None)
        .file(common_bridge)
        .std("c11")
        .warnings(false)
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .flag_if_supported("-Wno-unused-parameter");

    let config_define = quoted_path(&mbedtls_config);
    build.define("MBEDTLS_CONFIG_FILE", Some(config_define.as_str()));

    if is_scarlet {
        build
            .define("LC_SCARLET", None)
            .define("HAS_POLL", None)
            .define("HAS_INET_PTON", None)
            .define("HAS_INET_NTOP", None)
            .define("HAS_GETADDRINFO", None)
            .define("NO_MSGAPI", None)
            .flag("-fno-builtin")
            .flag("-fno-stack-protector")
            .file(scarlet_platform.join("compat.c"));
        configure_scarlet_arch(&mut build);
    }

    for source in c_files(&source_dir.join("src")) {
        build.file(source);
    }
    for name in [
        "callbacks.c",
        "compress.c",
        "host.c",
        "list.c",
        "packet.c",
        "peer.c",
        "protocol.c",
        "unix.c",
    ] {
        build.file(source_dir.join("enet").join(name));
    }
    for name in [
        "nanors/rs.c",
        "nanors/deps/obl/oblas_common.c",
        "nanors/deps/obl/oblas_lite.c",
    ] {
        build.file(source_dir.join(name));
    }

    build.compile("moonlight-common-c");
}

fn build_mbedcrypto(source_dir: &Path, config: &Path, platform_include: Option<&Path>) {
    let mut build = cc::Build::new();
    if let Some(platform_include) = platform_include {
        build.include(platform_include);
    }
    build
        .include(source_dir.join("include"))
        .include(source_dir.join("library"))
        .define("MBEDTLS_CONFIG_FILE", Some(quoted_path(config).as_str()))
        .std("c11")
        .warnings(false);

    if platform_include.is_some() {
        build
            .define("LC_SCARLET", None)
            .flag("-fno-builtin")
            .flag("-fno-stack-protector");
        configure_scarlet_arch(&mut build);
    }

    for source in MBEDTLS_CRYPTO_SOURCES {
        build.file(source_dir.join("library").join(source));
    }
    build.compile("mbedcrypto");
}

fn configure_scarlet_arch(build: &mut cc::Build) {
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("riscv64") {
        build.flag("-march=rv64gc").flag("-mabi=lp64d");
    }
}

fn quoted_path(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn c_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = fs::read_dir(directory)
        .expect("read moonlight-common-c source directory")
        .map(|entry| entry.expect("read moonlight-common-c source entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

const MBEDTLS_CRYPTO_SOURCES: &[&str] = &[
    "aes.c",
    "aesni.c",
    "aesce.c",
    "aria.c",
    "asn1parse.c",
    "asn1write.c",
    "base64.c",
    "bignum.c",
    "bignum_core.c",
    "bignum_mod.c",
    "bignum_mod_raw.c",
    "block_cipher.c",
    "camellia.c",
    "ccm.c",
    "chacha20.c",
    "chachapoly.c",
    "cipher.c",
    "cipher_wrap.c",
    "constant_time.c",
    "cmac.c",
    "ctr_drbg.c",
    "des.c",
    "dhm.c",
    "ecdh.c",
    "ecdsa.c",
    "ecjpake.c",
    "ecp.c",
    "ecp_curves.c",
    "entropy.c",
    "entropy_poll.c",
    "error.c",
    "gcm.c",
    "hkdf.c",
    "hmac_drbg.c",
    "lmots.c",
    "lms.c",
    "md.c",
    "md5.c",
    "memory_buffer_alloc.c",
    "nist_kw.c",
    "oid.c",
    "padlock.c",
    "pem.c",
    "pk.c",
    "pk_ecc.c",
    "pk_wrap.c",
    "pkcs12.c",
    "pkcs5.c",
    "pkparse.c",
    "pkwrite.c",
    "platform.c",
    "platform_util.c",
    "poly1305.c",
    "psa_crypto.c",
    "psa_crypto_aead.c",
    "psa_crypto_cipher.c",
    "psa_crypto_client.c",
    "psa_crypto_driver_wrappers_no_static.c",
    "psa_crypto_ecp.c",
    "psa_crypto_ffdh.c",
    "psa_crypto_hash.c",
    "psa_crypto_mac.c",
    "psa_crypto_pake.c",
    "psa_crypto_random.c",
    "psa_crypto_rsa.c",
    "psa_crypto_se.c",
    "psa_crypto_slot_management.c",
    "psa_crypto_storage.c",
    "psa_its_file.c",
    "psa_util.c",
    "ripemd160.c",
    "rsa.c",
    "rsa_alt_helpers.c",
    "sha1.c",
    "sha256.c",
    "sha512.c",
    "sha3.c",
    "threading.c",
    "timing.c",
    "version.c",
    "version_features.c",
];

fn emit_rerun_directives(directory: &Path) {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("walk moonlight-common-c source") {
            let path = entry.expect("read moonlight-common-c path").path();
            if path.is_dir() {
                pending.push(path);
            } else if matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("c" | "h")
            ) {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}
