//! License notices displayed by the Moonlight application.

/// One group of components distributed under the same license terms.
pub(crate) struct LicenseNotice {
    /// Short heading shown in the component list.
    pub(crate) title: &'static str,
    /// Components covered by this notice.
    pub(crate) components: &'static str,
    /// SPDX-style license description.
    pub(crate) license: &'static str,
    texts: &'static [&'static str],
}

const MOONLIGHT_LICENSES: &[&str] = &[include_str!("../../LICENSE")];
const OPUS_LICENSES: &[&str] = &[include_str!("../../licenses/opus-BSD-3-Clause.txt")];
const RUST_OPUS_LICENSES: &[&str] = &[
    include_str!("../../licenses/rust-opus-Apache-2.0.txt"),
    include_str!("../../licenses/rust-opus-NOTICE.txt"),
];
const MBED_TLS_LICENSES: &[&str] = &[include_str!("../../licenses/rust-opus-Apache-2.0.txt")];
const ENET_LICENSES: &[&str] = &[include_str!(
    "../../third_party/moonlight-common-c/enet/LICENSE"
)];
const NANORS_LICENSES: &[&str] = &[include_str!(
    "../../third_party/moonlight-common-c/nanors/LICENSE"
)];
const SCARLET_LICENSES: &[&str] = &[include_str!("../../licenses/scarlet-MIT.txt")];
const SCARLET_UI_LICENSES: &[&str] = &[include_str!("../../licenses/scarlet-ui-MIT.txt")];
const TABLER_ICONS_LICENSES: &[&str] = &[include_str!("../../licenses/tabler-icons-MIT.txt")];

/// Core component notices available from the in-application license browser.
pub(crate) const NOTICES: &[LicenseNotice] = &[
    LicenseNotice {
        title: "Moonlight Scarlet",
        components: "Moonlight Scarlet · moonlight-common-c",
        license: "GNU GPL version 3",
        texts: MOONLIGHT_LICENSES,
    },
    LicenseNotice {
        title: "Opus",
        components: "libopus",
        license: "BSD-3-Clause",
        texts: OPUS_LICENSES,
    },
    LicenseNotice {
        title: "rust-opus",
        components: "rust-opus · opus-head-sys",
        license: "Apache-2.0",
        texts: RUST_OPUS_LICENSES,
    },
    LicenseNotice {
        title: "Mbed TLS",
        components: "Mbed TLS · tstrans-mbedtls-src",
        license: "Apache-2.0",
        texts: MBED_TLS_LICENSES,
    },
    LicenseNotice {
        title: "ENet",
        components: "Moonlight bundled ENet",
        license: "MIT",
        texts: ENET_LICENSES,
    },
    LicenseNotice {
        title: "nanors",
        components: "moonlight-common-c nanors",
        license: "MIT",
        texts: NANORS_LICENSES,
    },
    LicenseNotice {
        title: "Scarlet",
        components: "Scarlet platform libraries",
        license: "MIT",
        texts: SCARLET_LICENSES,
    },
    LicenseNotice {
        title: "ScarletUI",
        components: "ScarletUI",
        license: "MIT",
        texts: SCARLET_UI_LICENSES,
    },
    LicenseNotice {
        title: "Tabler Icons",
        components: "Tabler Icons · glyphs used by scarlet-ui-icons-tabler",
        license: "MIT · Copyright (c) 2020-2026 Paweł Kuna",
        texts: TABLER_ICONS_LICENSES,
    },
];

/// Return a notice by index, falling back to the application notice.
///
/// # Arguments
///
/// * `index` - Component index selected in the license browser.
///
/// # Returns
///
/// The requested notice, or the Moonlight Scarlet notice for an invalid index.
pub(crate) fn notice(index: usize) -> &'static LicenseNotice {
    NOTICES.get(index).unwrap_or(&NOTICES[0])
}

/// Build wrapped display lines for a license detail page.
///
/// # Arguments
///
/// * `index` - Component index selected in the license browser.
/// * `maximum_characters` - Soft maximum line width in Unicode scalar values.
///
/// # Returns
///
/// Header, attribution, and license text split into virtualized display rows.
pub(crate) fn display_lines(index: usize, maximum_characters: usize) -> Vec<String> {
    let notice = notice(index);
    let mut sections = vec![
        notice.title.to_owned(),
        notice.components.to_owned(),
        format!("License: {}", notice.license),
        String::new(),
    ];
    for (text_index, text) in notice.texts.iter().enumerate() {
        if text_index != 0 {
            sections.push(String::new());
            sections.push(String::from("ADDITIONAL LICENSE OR NOTICE"));
            sections.push(String::new());
        }
        sections.extend(wrap_text(text, maximum_characters));
    }
    sections
}

fn wrap_text(text: &str, maximum_characters: usize) -> Vec<String> {
    let maximum_characters = maximum_characters.max(1);
    let mut output = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            output.push(String::new());
            continue;
        }

        let mut current = String::new();
        for word in line.split_whitespace() {
            let separator = usize::from(!current.is_empty());
            if !current.is_empty()
                && current.chars().count() + separator + word.chars().count() > maximum_characters
            {
                output.push(current);
                current = String::new();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        output.push(current);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{NOTICES, display_lines};

    #[test]
    fn every_notice_contains_license_text() {
        assert_eq!(NOTICES.len(), 9);
        assert!(NOTICES.iter().all(|notice| {
            !notice.title.is_empty()
                && !notice.license.is_empty()
                && notice.texts.iter().all(|text| !text.trim().is_empty())
        }));
    }

    #[test]
    fn long_license_paragraphs_are_wrapped_for_the_viewport() {
        let lines = display_lines(3, 48);
        assert!(lines.len() > 10);
        assert!(
            lines.iter().all(|line| {
                line.chars().count() <= 48 || !line.chars().any(char::is_whitespace)
            })
        );
    }
}
