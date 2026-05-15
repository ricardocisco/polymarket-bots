use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

/// Deserializa timestamp que pode ser int OU float (TypeScript armazena como float)
fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("integer or float timestamp")
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<i64, E> {
            Ok(v)
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<i64, E> {
            Ok(v as i64)
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<i64, E> {
            Ok(v as i64)
        }
    }
    deserializer.deserialize_any(Visitor)
}

/// Filtros por subscription — mesmos do TypeScript (minUsd + keywords)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SubscriptionFilters {
    pub keywords: Option<Vec<String>>,
    /// Valor mínimo em USD para notificar (0 = sem filtro)
    #[serde(rename = "minUsd")]
    pub min_usd: Option<f64>,
}

/// Carteira monitorada — compatível com o schema Mongoose existente
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Wallet {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub address: String,
    /// Unix timestamp. TypeScript armazena como float (ex: 1768419923395.0) —
    /// o deserializer aceita int e float e converte para i64.
    #[serde(rename = "lastTimestamp", deserialize_with = "deserialize_timestamp")]
    pub last_timestamp: i64,
}

/// Inscrição (canal Discord → carteira) — compatível com o schema Mongoose existente
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Subscription {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    #[serde(rename = "channelId")]
    pub channel_id: String,
    #[serde(rename = "walletAddress")]
    pub wallet_address: String,
    /// ID do usuário Discord que criou o tracking
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    pub filters: Option<SubscriptionFilters>,
}
