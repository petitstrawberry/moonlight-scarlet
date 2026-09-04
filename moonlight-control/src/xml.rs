//! XML parsing for the GameStream HTTP protocol.

use roxmltree::{Document, Node};

use crate::client::ControlError;
use crate::{Application, ServerInfo};

pub(crate) fn parse_server_info(xml: &str) -> Result<ServerInfo, ControlError> {
    let document = checked_document(xml)?;
    let state = required_text(&document, "state")?.to_owned();
    let current_game = if state.ends_with("_SERVER_BUSY") {
        optional_text(&document, "currentgame")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    Ok(ServerInfo {
        hostname: required_text(&document, "hostname")?.to_owned(),
        unique_id: required_text(&document, "uniqueid")?.to_owned(),
        app_version: required_text(&document, "appversion")?.to_owned(),
        gfe_version: optional_text(&document, "GfeVersion").map(ToOwned::to_owned),
        server_codec_mode_support: optional_text(&document, "ServerCodecModeSupport")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1),
        https_port: optional_text(&document, "HttpsPort")
            .and_then(|value| value.parse().ok())
            .filter(|port| *port != 0)
            .unwrap_or(47_984),
        paired: required_text(&document, "PairStatus")? == "1",
        current_game,
        state,
    })
}

pub(crate) fn parse_applications(xml: &str) -> Result<Vec<Application>, ControlError> {
    let document = checked_document(xml)?;
    document
        .descendants()
        .filter(|node| node.has_tag_name("App"))
        .map(|app| {
            let id = child_text(app, "ID")?
                .parse::<u32>()
                .map_err(|_| ControlError::Xml("application ID is not an integer".to_owned()))?;
            let title = child_text(app, "AppTitle")?.to_owned();
            Ok(Application { id, title })
        })
        .collect()
}

pub(crate) fn response_text(xml: &str, tag: &str) -> Result<String, ControlError> {
    let document = checked_document(xml)?;
    Ok(required_text(&document, tag)?.to_owned())
}

pub(crate) fn response_hex(xml: &str, tag: &str) -> Result<Vec<u8>, ControlError> {
    let value = response_text(xml, tag)?;
    hex::decode(value).map_err(|error| ControlError::Xml(format!("invalid {tag} hex: {error}")))
}

pub(crate) fn require_paired(xml: &str, stage: &str) -> Result<(), ControlError> {
    if response_text(xml, "paired")? == "1" {
        Ok(())
    } else {
        Err(ControlError::Pairing(format!(
            "pairing was rejected during {stage}"
        )))
    }
}

fn checked_document(xml: &str) -> Result<Document<'_>, ControlError> {
    let document = Document::parse(xml).map_err(|error| ControlError::Xml(error.to_string()))?;
    let root = document.root_element();
    if !root.has_tag_name("root") {
        return Err(ControlError::Xml("response has no root element".to_owned()));
    }
    let status_code = root
        .attribute("status_code")
        .ok_or_else(|| ControlError::Xml("response has no status_code".to_owned()))?;
    let status_code = status_code
        .parse::<u32>()
        .map_err(|_| ControlError::Xml("status_code is not an integer".to_owned()))?;
    if status_code != 200 {
        let message = root.attribute("status_message").unwrap_or("unknown error");
        return Err(ControlError::Protocol {
            code: status_code,
            message: message.to_owned(),
        });
    }
    Ok(document)
}

fn required_text<'a>(document: &'a Document<'a>, tag: &str) -> Result<&'a str, ControlError> {
    optional_text(document, tag)
        .ok_or_else(|| ControlError::Xml(format!("response has no {tag} element")))
}

fn optional_text<'a>(document: &'a Document<'a>, tag: &str) -> Option<&'a str> {
    document
        .descendants()
        .find(|node| node.has_tag_name(tag))
        .and_then(|node| node.text())
}

fn child_text<'a, 'input>(parent: Node<'a, 'input>, tag: &str) -> Result<&'a str, ControlError> {
    parent
        .children()
        .find(|node| node.has_tag_name(tag))
        .and_then(|node| node.text())
        .ok_or_else(|| ControlError::Xml(format!("application has no {tag} element")))
}

#[cfg(test)]
mod tests {
    use super::{parse_applications, parse_server_info};

    #[test]
    fn parses_sunshine_server_info() {
        let info = parse_server_info(
            r#"<?xml version="1.0"?><root status_code="200"><hostname>Studio</hostname><appversion>7.1.431.-1</appversion><GfeVersion>3.23.0.74</GfeVersion><uniqueid>ABC</uniqueid><HttpsPort>47984</HttpsPort><ServerCodecModeSupport>1835777</ServerCodecModeSupport><PairStatus>0</PairStatus><currentgame>0</currentgame><state>SUNSHINE_SERVER_FREE</state></root>"#,
        )
        .expect("server info");

        assert_eq!(info.hostname, "Studio");
        assert_eq!(info.server_major_version().expect("major version"), 7);
        assert_eq!(info.server_codec_mode_support, 1_835_777);
        assert!(!info.paired);
    }

    #[test]
    fn parses_application_list() {
        let apps = parse_applications(
            r#"<root status_code="200"><App><AppTitle>Desktop</AppTitle><ID>1</ID></App><App><AppTitle>Steam &amp; Friends</AppTitle><ID>2</ID></App></root>"#,
        )
        .expect("application list");

        assert_eq!(apps.len(), 2);
        assert_eq!(apps[1].title, "Steam & Friends");
    }

    #[test]
    fn ignores_stale_current_game_when_server_is_free() {
        let info = parse_server_info(
            r#"<root status_code="200"><hostname>Studio</hostname><appversion>7.1.431.-1</appversion><uniqueid>ABC</uniqueid><PairStatus>1</PairStatus><currentgame>42</currentgame><state>SUNSHINE_SERVER_FREE</state></root>"#,
        )
        .expect("server info");

        assert_eq!(info.current_game, 0);
    }

    #[test]
    fn reports_current_game_when_server_is_busy() {
        let info = parse_server_info(
            r#"<root status_code="200"><hostname>Studio</hostname><appversion>7.1.431.-1</appversion><uniqueid>ABC</uniqueid><PairStatus>1</PairStatus><currentgame>42</currentgame><state>SUNSHINE_SERVER_BUSY</state></root>"#,
        )
        .expect("server info");

        assert_eq!(info.current_game, 42);
    }

    #[test]
    fn defaults_legacy_hosts_to_h264_codec_support() {
        let info = parse_server_info(
            r#"<root status_code="200"><hostname>Legacy</hostname><appversion>7.1.431.-1</appversion><uniqueid>ABC</uniqueid><PairStatus>1</PairStatus><currentgame>0</currentgame><state>SUNSHINE_SERVER_FREE</state></root>"#,
        )
        .expect("server info");

        assert_eq!(info.server_codec_mode_support, 1);
    }
}
