use serde::Serialize;

use crate::i18n::{t, Lang};

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub key: String,
    pub message: String,
}

impl AppError {
    pub fn new(key: impl Into<String>, lang: Lang) -> Self {
        let key = key.into();
        let message = t(lang, &key).to_string();
        Self { key, message }
    }

    pub fn to_string(&self) -> String {
        self.message.clone()
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
