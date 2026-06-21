use std::fmt;

use actix_session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashType {
    Success,
    Error,
    Warning,
    Info,
}

impl FlashType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FlashType::Success => "success",
            FlashType::Error => "error",
            FlashType::Warning => "warning",
            FlashType::Info => "info",
        }
    }
}

impl fmt::Display for FlashType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for FlashType {
    type Error = String;

    fn try_from(s: String) -> Result<Self, <Self as TryFrom<String>>::Error> {
        match s.as_str() {
            "success" => Ok(FlashType::Success),
            "error" => Ok(FlashType::Error),
            "warning" => Ok(FlashType::Warning),
            "info" => Ok(FlashType::Info),
            other => Err(format!("unknown flash type: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlashMessage {
    pub message: String,
    pub flash_type: FlashType,
}

pub fn set_flash(session: &Session, message: &str, flash_type: FlashType) {
    let _ = session.insert("flash_message", message);
    let _ = session.insert("flash_type", flash_type.as_str());
}

pub fn take_flash(session: &Session) -> Option<FlashMessage> {
    let message = session.remove("flash_message")?;
    let flash_type_str: String = session
        .remove("flash_type")
        .unwrap_or_else(|| "success".to_string());
    let flash_type = FlashType::try_from(flash_type_str).unwrap_or(FlashType::Success);
    Some(FlashMessage {
        message,
        flash_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_message_struct() {
        let flash = FlashMessage {
            message: "Mod installed".to_string(),
            flash_type: FlashType::Success,
        };
        assert_eq!(flash.message, "Mod installed");
        assert_eq!(flash.flash_type, FlashType::Success);
    }

    #[test]
    fn flash_type_as_str() {
        assert_eq!(FlashType::Success.as_str(), "success");
        assert_eq!(FlashType::Error.as_str(), "error");
        assert_eq!(FlashType::Warning.as_str(), "warning");
        assert_eq!(FlashType::Info.as_str(), "info");
    }

    #[test]
    fn flash_type_display() {
        assert_eq!(format!("{}", FlashType::Success), "success");
        assert_eq!(format!("{}", FlashType::Error), "error");
    }

    #[test]
    fn flash_type_try_from() {
        assert_eq!(
            FlashType::try_from("success".to_string()),
            Ok(FlashType::Success)
        );
        assert_eq!(
            FlashType::try_from("error".to_string()),
            Ok(FlashType::Error)
        );
        assert!(FlashType::try_from("invalid".to_string()).is_err());
    }
}
