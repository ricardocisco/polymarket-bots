// src/storage/weather_paper_trades.rs
//! Store de paper trades de clima no banco de dados `first_minute_followthrough`.
//!
//! Tabela: `weather_paper_trades`
//! Reutiliza o mesmo banco do projeto first-minute-followthrough-rust para
//! centralizar todos os logs de trading simulado em um único lugar.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    ClientConfig as RustlsClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};
use tokio_postgres::{config::SslMode, Client, Config as PgConfig};
use tokio_rustls::TlsConnector;

const CREATE_TABLE_SQL: &str = "
CREATE TABLE IF NOT EXISTS weather_paper_trades (
  id                BIGSERIAL PRIMARY KEY,
  entry_id          TEXT        NOT NULL,
  city              TEXT        NOT NULL,
  target_date       TEXT        NOT NULL,
  icao              TEXT        NOT NULL,
  resolution_source TEXT        NOT NULL,
  question          TEXT        NOT NULL,
  direction         TEXT        NOT NULL,
  token_id          TEXT        NOT NULL,
  size_usdc         DOUBLE PRECISION NOT NULL,
  entry_price       DOUBLE PRECISION NOT NULL,
  predicted_temp    DOUBLE PRECISION NOT NULL,
  temp_unit         TEXT        NOT NULL,
  confidence        DOUBLE PRECISION NOT NULL,
  effective_confidence DOUBLE PRECISION,
  expected_value    DOUBLE PRECISION,
  edge_per_share    DOUBLE PRECISION,
  strategy_type     TEXT,
  execution_mode    TEXT        NOT NULL DEFAULT 'paper',
  order_id           TEXT,
  status            TEXT        NOT NULL DEFAULT 'pending',
  actual_temp       DOUBLE PRECISION,
  pnl               DOUBLE PRECISION,
  resolved_at       TEXT,
  winning_question  TEXT,
  registered_at     TEXT        NOT NULL,
  created_at        BIGINT      NOT NULL,
  CONSTRAINT weather_paper_trades_entry_id_key UNIQUE (entry_id)
);

CREATE INDEX IF NOT EXISTS idx_weather_paper_trades_status
  ON weather_paper_trades(status, target_date);

CREATE INDEX IF NOT EXISTS idx_weather_paper_trades_city
  ON weather_paper_trades(city, target_date);

ALTER TABLE weather_paper_trades
  ADD COLUMN IF NOT EXISTS effective_confidence DOUBLE PRECISION;

ALTER TABLE weather_paper_trades
  ADD COLUMN IF NOT EXISTS expected_value DOUBLE PRECISION;

ALTER TABLE weather_paper_trades
  ADD COLUMN IF NOT EXISTS edge_per_share DOUBLE PRECISION;

ALTER TABLE weather_paper_trades
  ADD COLUMN IF NOT EXISTS strategy_type TEXT;

ALTER TABLE weather_paper_trades
  ADD COLUMN IF NOT EXISTS execution_mode TEXT NOT NULL DEFAULT 'paper';

ALTER TABLE weather_paper_trades
  ADD COLUMN IF NOT EXISTS order_id TEXT;
";

#[derive(Clone)]
pub struct WeatherTradeStore {
    client: Arc<Client>,
}

#[derive(Debug, Clone)]
pub struct WeatherTradeRow {
    pub entry_id: String,
    pub city: String,
    pub target_date: String,
    pub icao: String,
    pub resolution_source: String,
    pub question: String,
    pub direction: String,
    pub token_id: String,
    pub size_usdc: f64,
    pub entry_price: f64,
    pub predicted_temp: f64,
    pub temp_unit: String,
    pub confidence: f64,
    pub effective_confidence: Option<f64>,
    pub expected_value: Option<f64>,
    pub edge_per_share: Option<f64>,
    pub strategy_type: Option<String>,
    pub execution_mode: String,
    pub order_id: Option<String>,
    pub status: String,
    pub actual_temp: Option<f64>,
    pub pnl: Option<f64>,
    pub resolved_at: Option<String>,
    pub winning_question: Option<String>,
    pub registered_at: String,
    pub created_at: i64,
}

impl WeatherTradeStore {
    /// Conecta ao banco de dados e garante que a tabela existe.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let sanitized = sanitize_database_url(database_url);
        let mut config =
            PgConfig::from_str(&sanitized).context("DATABASE_URL inválida para tokio-postgres")?;

        // Supabase exige TLS — garante que o modo SSL esteja definido como Require
        // independente do que vier na URL.
        config.ssl_mode(SslMode::Require);

        let tls = PostgresRustlsConnector::new();

        let (client, connection) = config
            .connect(tls)
            .await
            .context("falha ao conectar no Postgres")?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("[weather_trade_db] conexão encerrada com erro: {e}");
            }
        });

        client
            .batch_execute(CREATE_TABLE_SQL)
            .await
            .context("falha ao garantir schema de weather_paper_trades")?;

        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// Insere uma nova entrada como "pending". Idempotente via ON CONFLICT DO NOTHING.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_open_trade(
        &self,
        entry_id: &str,
        city: &str,
        target_date: &str,
        icao: &str,
        resolution_source: &str,
        question: &str,
        direction: &str,
        token_id: &str,
        size_usdc: f64,
        entry_price: f64,
        predicted_temp: f64,
        temp_unit: &str,
        confidence: f64,
        effective_confidence: Option<f64>,
        expected_value: Option<f64>,
        edge_per_share: Option<f64>,
        strategy_type: Option<&str>,
        execution_mode: &str,
        order_id: Option<&str>,
        registered_at: &str,
        created_at: i64,
    ) -> Result<()> {
        self.client
            .execute(
                "INSERT INTO weather_paper_trades (
                    entry_id, city, target_date, icao, resolution_source,
                    question, direction, token_id,
                    size_usdc, entry_price, predicted_temp, temp_unit, confidence,
                    effective_confidence, expected_value, edge_per_share, strategy_type,
                    execution_mode, order_id,
                    status, registered_at, created_at
                 ) VALUES (
                    $1, $2, $3, $4, $5,
                    $6, $7, $8,
                    $9, $10, $11, $12, $13,
                    $14, $15, $16, $17,
                    $18, $19,
                    'pending', $20, $21
                 ) ON CONFLICT (entry_id) DO NOTHING",
                &[
                    &entry_id,
                    &city,
                    &target_date,
                    &icao,
                    &resolution_source,
                    &question,
                    &direction,
                    &token_id,
                    &size_usdc,
                    &entry_price,
                    &predicted_temp,
                    &temp_unit,
                    &confidence,
                    &effective_confidence,
                    &expected_value,
                    &edge_per_share,
                    &strategy_type,
                    &execution_mode,
                    &order_id,
                    &registered_at,
                    &created_at,
                ],
            )
            .await
            .context("falha ao inserir weather paper trade")?;

        Ok(())
    }

    /// Atualiza uma entrada com o resultado da resolução.
    /// Preenche `actual_temp` com a temperatura oficial usada na resolução.
    pub async fn settle_trade(
        &self,
        entry_id: &str,
        status: &str,
        actual_temp: Option<f64>,
        pnl: f64,
        winning_question: &str,
        resolved_at: &str,
    ) -> Result<()> {
        self.client
            .execute(
                "UPDATE weather_paper_trades
                    SET status           = $1,
                        actual_temp      = $2,
                        pnl              = $3,
                        winning_question = $4,
                        resolved_at      = $5
                  WHERE entry_id = $6
                    AND status   = 'pending'",
                &[
                    &status,
                    &actual_temp,
                    &pnl,
                    &winning_question,
                    &resolved_at,
                    &entry_id,
                ],
            )
            .await
            .context("falha ao liquidar weather paper trade")?;

        Ok(())
    }

    /// Retorna os entry_ids de trades que já estão no banco (para sincronizar o book JSON).
    pub async fn existing_entry_ids(&self) -> Result<Vec<String>> {
        let rows = self
            .client
            .query("SELECT entry_id FROM weather_paper_trades", &[])
            .await
            .context("falha ao buscar entry_ids existentes")?;

        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }

    /// Carrega todos os trades persistidos, ordenados por criação.
    pub async fn list_trades(&self) -> Result<Vec<WeatherTradeRow>> {
        let rows = self
            .client
            .query(
                "SELECT
                    entry_id,
                    city,
                    target_date,
                    icao,
                    resolution_source,
                    question,
                    direction,
                    token_id,
                    size_usdc,
                    entry_price,
                    predicted_temp,
                    temp_unit,
                    confidence,
                    effective_confidence,
                    expected_value,
                    edge_per_share,
                    strategy_type,
                    execution_mode,
                    order_id,
                    status,
                    actual_temp,
                    pnl,
                    resolved_at,
                    winning_question,
                    registered_at,
                    created_at
                 FROM weather_paper_trades
                 ORDER BY created_at ASC, id ASC",
                &[],
            )
            .await
            .context("falha ao carregar weather paper trades")?;

        Ok(rows
            .into_iter()
            .map(|row| WeatherTradeRow {
                entry_id: row.get("entry_id"),
                city: row.get("city"),
                target_date: row.get("target_date"),
                icao: row.get("icao"),
                resolution_source: row.get("resolution_source"),
                question: row.get("question"),
                direction: row.get("direction"),
                token_id: row.get("token_id"),
                size_usdc: row.get("size_usdc"),
                entry_price: row.get("entry_price"),
                predicted_temp: row.get("predicted_temp"),
                temp_unit: row.get("temp_unit"),
                confidence: row.get("confidence"),
                effective_confidence: row.get("effective_confidence"),
                expected_value: row.get("expected_value"),
                edge_per_share: row.get("edge_per_share"),
                strategy_type: row.get("strategy_type"),
                execution_mode: row.get("execution_mode"),
                order_id: row.get("order_id"),
                status: row.get("status"),
                actual_temp: row.get("actual_temp"),
                pnl: row.get("pnl"),
                resolved_at: row.get("resolved_at"),
                winning_question: row.get("winning_question"),
                registered_at: row.get("registered_at"),
                created_at: row.get("created_at"),
            })
            .collect())
    }
}

// ── Helpers ───────────────────────────────────────────────────

fn sanitize_database_url(database_url: &str) -> String {
    if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
        return database_url.to_string();
    }

    let Some((base, query)) = database_url.split_once('?') else {
        return database_url.to_string();
    };

    let mut supported_params = Vec::new();

    for pair in query.split('&').filter(|p| !p.trim().is_empty()) {
        let key = pair.split_once('=').map(|(k, _)| k).unwrap_or(pair);
        if is_supported_postgres_option(key) {
            supported_params.push(pair);
        }
    }

    if supported_params.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", supported_params.join("&"))
    }
}

fn is_supported_postgres_option(key: &str) -> bool {
    matches!(
        key,
        "user"
            | "password"
            | "dbname"
            | "options"
            | "application_name"
            | "sslmode"
            | "sslnegotiation"
            | "host"
            | "hostaddr"
            | "port"
            | "connect_timeout"
            | "tcp_user_timeout"
            | "keepalives"
            | "keepalives_idle"
            | "keepalives_interval"
            | "keepalives_retries"
            | "target_session_attrs"
            | "channel_binding"
            | "load_balance_hosts"
    )
}

// ── TLS via rustls (sem dependência de OpenSSL) ───────────────

// Verificador que aceita qualquer certificado do servidor, equivalente ao
// sslmode=require do libpq: a conexão é criptografada, mas a identidade do
// certificado não é verificada. Necessário para o pooler do Supabase
// (PgBouncer), que apresenta um certificado assinado por CA própria.
#[derive(Debug)]
struct AcceptAnyCert;

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

#[derive(Clone)]
struct PostgresRustlsConnector {
    config: Arc<RustlsClientConfig>,
}

impl PostgresRustlsConnector {
    fn new() -> Self {
        // Não definir ALPN: o pooler do Supabase (PgBouncer) não suporta ALPN
        // para PostgreSQL e rejeita o handshake TLS quando enviado.
        let config = RustlsClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
            .with_no_client_auth();

        Self {
            config: Arc::new(config),
        }
    }
}

impl<S> MakeTlsConnect<S> for PostgresRustlsConnector
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = PostgresRustlsStream<S>;
    type TlsConnect = PostgresRustlsConnect;
    type Error = io::Error;

    fn make_tls_connect(&mut self, domain: &str) -> io::Result<Self::TlsConnect> {
        let server_name = if domain.is_empty() {
            None
        } else {
            Some(ServerName::try_from(domain.to_string()).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("hostname TLS inválido: {e}"),
                )
            })?)
        };

        Ok(PostgresRustlsConnect {
            connector: TlsConnector::from(Arc::clone(&self.config)),
            server_name,
        })
    }
}

struct PostgresRustlsConnect {
    connector: TlsConnector,
    server_name: Option<ServerName<'static>>,
}

impl<S> TlsConnect<S> for PostgresRustlsConnect
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = PostgresRustlsStream<S>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<Self::Stream>> + Send>>;

    fn connect(self, stream: S) -> Self::Future {
        Box::pin(async move {
            let server_name = self.server_name.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "hostname ausente para handshake TLS",
                )
            })?;

            let tls = self
                .connector
                .connect(server_name, stream)
                .await
                .map_err(|e| io::Error::other(format!("falha no handshake TLS: {e}")))?;

            Ok(PostgresRustlsStream(tls))
        })
    }
}

struct PostgresRustlsStream<S>(tokio_rustls::client::TlsStream<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for PostgresRustlsStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for PostgresRustlsStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> TlsStream for PostgresRustlsStream<S> {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}
