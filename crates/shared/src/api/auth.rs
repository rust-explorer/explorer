use serde::{Deserialize, Serialize};

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
