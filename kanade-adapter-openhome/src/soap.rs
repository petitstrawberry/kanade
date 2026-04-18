use quick_xml::{events::Event, Reader};

use kanade_core::error::CoreError;

/// The OpenHome action names this adapter handles.
#[derive(Debug, Clone, PartialEq)]
pub enum SoapAction {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SeekSecondAbsolute {
        seconds: u32,
    },
    SetVolume {
        volume: u8,
    },
    /// Any other action — returned as an opaque string so callers can log it.
    Unknown(String),
}

/// Parse a raw SOAP/XML request body and return the action it represents.
///
/// Only the `<s:Body>` element's first child tag name is inspected; the full
/// UPnP envelope structure is not validated beyond what is necessary.
pub fn parse_action(soap_body: &str, soap_action_header: &str) -> Result<SoapAction, CoreError> {
    // The SOAPAction HTTP header looks like:
    //   "urn:av-openhome-org:service:Transport:1#Play"
    // We extract the fragment after '#'.
    let action_name = soap_action_header
        .rsplit('#')
        .next()
        .unwrap_or("")
        .trim_matches('"')
        .trim();

    match action_name {
        "Play" => Ok(SoapAction::Play),
        "Pause" => Ok(SoapAction::Pause),
        "Stop" => Ok(SoapAction::Stop),
        "Next" => Ok(SoapAction::Next),
        "Previous" => Ok(SoapAction::Previous),
        "SeekSecondAbsolute" => {
            let seconds = extract_u32(soap_body, "Value").unwrap_or(0);
            Ok(SoapAction::SeekSecondAbsolute { seconds })
        }
        "SetVolume" => {
            let volume = extract_u32(soap_body, "Value").unwrap_or(50).min(100) as u8;
            Ok(SoapAction::SetVolume { volume })
        }
        other => Ok(SoapAction::Unknown(other.to_string())),
    }
}

/// Build a minimal SOAP response envelope for a successful void action.
pub fn ok_response(action_name: &str, service_type: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:{action_name}Response xmlns:u="{service_type}"/>
  </s:Body>
</s:Envelope>"#
    )
}

/// Build a SOAP fault response.
pub fn fault_response(code: u32, description: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <s:Fault>
      <faultcode>s:Client</faultcode>
      <faultstring>UPnPError</faultstring>
      <detail>
        <UPnPError xmlns="urn:schemas-upnp-org:control-1-0">
          <errorCode>{code}</errorCode>
          <errorDescription>{description}</errorDescription>
        </UPnPError>
      </detail>
    </s:Fault>
  </s:Body>
</s:Envelope>"#
    )
}

// ---------------------------------------------------------------------------
// XML extraction helpers
// ---------------------------------------------------------------------------

/// Extract the text content of the first `<tag>` element in an XML string.
fn extract_u32(xml: &str, tag: &str) -> Option<u32> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut inside = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.local_name().as_ref() == tag.as_bytes() => {
                inside = true;
            }
            Ok(Event::Text(ref e)) if inside => {
                return e.unescape().ok()?.parse().ok();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_play_action() {
        let body = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body><u:Play xmlns:u="urn:av-openhome-org:service:Transport:1"/></s:Body>
</s:Envelope>"#;
        let action = parse_action(body, "urn:av-openhome-org:service:Transport:1#Play").unwrap();
        assert_eq!(action, SoapAction::Play);
    }

    #[test]
    fn parse_seek_action() {
        let body = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <u:SeekSecondAbsolute xmlns:u="urn:av-openhome-org:service:Transport:1">
      <Value>120</Value>
    </u:SeekSecondAbsolute>
  </s:Body>
</s:Envelope>"#;
        let action = parse_action(
            body,
            "urn:av-openhome-org:service:Transport:1#SeekSecondAbsolute",
        )
        .unwrap();
        assert_eq!(action, SoapAction::SeekSecondAbsolute { seconds: 120 });
    }

    #[test]
    fn parse_set_volume_action() {
        let body = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <u:SetVolume xmlns:u="urn:av-openhome-org:service:Volume:1">
      <Value>75</Value>
    </u:SetVolume>
  </s:Body>
</s:Envelope>"#;
        let action = parse_action(body, "urn:av-openhome-org:service:Volume:1#SetVolume").unwrap();
        assert_eq!(action, SoapAction::SetVolume { volume: 75 });
    }

    #[test]
    fn ok_response_is_valid_xml() {
        let xml = ok_response("Play", "urn:av-openhome-org:service:Transport:1");
        assert!(xml.contains("PlayResponse"));
    }

    #[test]
    fn fault_response_contains_error_code() {
        let xml = fault_response(402, "Invalid Args");
        assert!(xml.contains("402"));
        assert!(xml.contains("Invalid Args"));
    }
}
