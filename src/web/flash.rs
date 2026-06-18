use actix_session::Session;

#[derive(Debug, Clone)]
pub struct FlashMessage {
    pub message: String,
    pub flash_type: String,
}

pub fn set_flash(session: &Session, message: &str, flash_type: &str) {
    let _ = session.insert("flash_message", message);
    let _ = session.insert("flash_type", flash_type);
}

pub fn take_flash(session: &Session) -> Option<FlashMessage> {
    let message = session.remove("flash_message")?;
    let flash_type = session
        .remove("flash_type")
        .unwrap_or_else(|| "success".to_string());
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
            flash_type: "success".to_string(),
        };
        assert_eq!(flash.message, "Mod installed");
        assert_eq!(flash.flash_type, "success");
    }
}
