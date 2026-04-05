use regex::Regex;
use std::sync::LazyLock;

/// Escape a string for safe interpolation into XML attributes and text.
pub fn escape_xml(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(ch),
        }
    }
    output
}

static GRAPHICS_TYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<graphics\s+type=['"]([\w]+)['""]"#).unwrap());

static SERIAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<(serial|console)\s+type=["']"#).unwrap());

/// Extract the graphics type (vnc/spice) from domain XML.
pub fn extract_graphics_type(xml: &str) -> Option<String> {
    GRAPHICS_TYPE_RE
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Check if domain XML contains a serial console.
pub fn has_serial_console(xml: &str) -> bool {
    SERIAL_RE.is_match(xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_xml_special_chars() {
        assert_eq!(escape_xml(r#"a<b>c&d"e'f"#), "a&lt;b&gt;c&amp;d&quot;e&apos;f");
    }

    #[test]
    fn escape_xml_passthrough() {
        assert_eq!(escape_xml("hello world"), "hello world");
    }

    #[test]
    fn extract_graphics_vnc_double_quotes() {
        let xml = r#"<domain><devices><graphics type="vnc" port="-1"/></devices></domain>"#;
        assert_eq!(extract_graphics_type(xml), Some("vnc".into()));
    }

    #[test]
    fn extract_graphics_spice_single_quotes() {
        let xml = "<domain><devices><graphics type='spice' autoport='yes'/></devices></domain>";
        assert_eq!(extract_graphics_type(xml), Some("spice".into()));
    }

    #[test]
    fn extract_graphics_none() {
        let xml = r#"<domain><devices></devices></domain>"#;
        assert_eq!(extract_graphics_type(xml), None);
    }

    #[test]
    fn has_serial_console_double_quotes() {
        let xml = r#"<domain><devices><serial type="pty"/></devices></domain>"#;
        assert!(has_serial_console(xml));
    }

    #[test]
    fn has_serial_console_single_quotes() {
        let xml = "<domain><devices><console type='pty'/></devices></domain>";
        assert!(has_serial_console(xml));
    }

    #[test]
    fn has_serial_console_false() {
        let xml = r#"<domain><devices><graphics type="vnc"/></devices></domain>"#;
        assert!(!has_serial_console(xml));
    }
}
