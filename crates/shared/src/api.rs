use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub request_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Health {
    pub status: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Me {
    pub sub: String,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LogoutResponse {
    pub redirect_to: String,
}
