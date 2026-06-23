use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct GetHeadlessesResponse {
    #[serde(default)]
    pub headlesses: HashMap<String, HeadlessClientInfo>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HeadlessClientInfo {
    #[serde(alias = "State")]
    pub state: EHeadlessStatus,
    #[serde(alias = "Players", default)]
    pub players: Vec<String>,
    #[serde(alias = "RequesterSessionID", default)]
    pub requester_session_id: Option<String>,
    #[serde(alias = "HasNotifiedRequester", default)]
    pub has_notified_requester: Option<bool>,
    #[serde(alias = "Level", default)]
    pub level: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EHeadlessStatus {
    Ready,
    InRaid,
    Unknown(Value),
}

impl<'de> Deserialize<'de> for EHeadlessStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        match &v {
            Value::Number(n) => match n.as_u64() {
                Some(1) => Ok(EHeadlessStatus::Ready),
                Some(2) => Ok(EHeadlessStatus::InRaid),
                _ => Ok(EHeadlessStatus::Unknown(v)),
            },
            Value::String(s) => match s.as_str() {
                "READY" | "Ready" => Ok(EHeadlessStatus::Ready),
                "IN_RAID" | "InRaid" => Ok(EHeadlessStatus::InRaid),
                _ => Ok(EHeadlessStatus::Unknown(v)),
            },
            _ => Ok(EHeadlessStatus::Unknown(v)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_headless_status_numeric() {
        let v: EHeadlessStatus = serde_json::from_str("1").unwrap();
        assert_eq!(v, EHeadlessStatus::Ready);
        let v: EHeadlessStatus = serde_json::from_str("2").unwrap();
        assert_eq!(v, EHeadlessStatus::InRaid);
    }

    #[test]
    fn deserialize_headless_status_string() {
        let v: EHeadlessStatus = serde_json::from_str(r#""READY""#).unwrap();
        assert_eq!(v, EHeadlessStatus::Ready);
        let v: EHeadlessStatus = serde_json::from_str(r#""IN_RAID""#).unwrap();
        assert_eq!(v, EHeadlessStatus::InRaid);
    }

    #[test]
    fn deserialize_headlesses_response() {
        let json = r#"{
            "headlesses": {
                "abc123": {
                    "State": 1,
                    "Players": [],
                    "RequesterSessionID": null,
                    "HasNotifiedRequester": null,
                    "Level": 0
                },
                "def456": {
                    "State": 2,
                    "Players": ["player1", "player2"],
                    "RequesterSessionID": "req789",
                    "HasNotifiedRequester": true,
                    "Level": 15
                }
            }
        }"#;
        let resp: GetHeadlessesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.headlesses.len(), 2);
        let client = &resp.headlesses["abc123"];
        assert_eq!(client.state, EHeadlessStatus::Ready);
        assert!(client.players.is_empty());
        let raiding = &resp.headlesses["def456"];
        assert_eq!(raiding.state, EHeadlessStatus::InRaid);
        assert_eq!(raiding.players.len(), 2);
        assert_eq!(raiding.requester_session_id.as_deref(), Some("req789"));
    }

    #[test]
    fn deserialize_empty_headlesses() {
        let json = r#"{"headlesses": {}}"#;
        let resp: GetHeadlessesResponse = serde_json::from_str(json).unwrap();
        assert!(resp.headlesses.is_empty());
    }
}
