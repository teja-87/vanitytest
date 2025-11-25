use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use chrono::{DateTime, Utc};
use std::sync::Arc;

// Helius webhook payload structures
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeliusWebhook {
    #[serde(default)]
    pub transaction: Option<TransactionData>,
    #[serde(default)]
    pub native_transfers: Option<Vec<NativeTransfer>>,
    pub timestamp: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionData {
    pub signature: String,
    pub slot: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeTransfer {
    pub from_user_account: String,
    pub to_user_account: String,
    pub amount: u64, // lamports
}

// Database model
#[derive(Debug, FromRow, Serialize)]
struct Transaction {
    pub id: i32,
    pub signature: String,
    pub sender: String,
    pub receiver: String,
    pub amount_lamports: i64,
    pub amount_sol: f64,
    pub timestamp: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// Application state
#[derive(Clone)]
struct AppState {
    db: PgPool,
}

// Response structure
#[derive(Serialize)]
struct WebhookResponse {
    success: bool,
    message: String,
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Database connection - replace with your connection string
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://user:password@localhost/solana_vanity".to_string());
    
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    let state = Arc::new(AppState { db: pool });

    // Build router
    let app = Router::new()
        .route("/webhook/helius", post(helius_webhook_handler))
        .route("/health", axum::routing::get(health_check))
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    
    println!("Server running on http://0.0.0.0:3000");
    
    axum::serve(listener, app).await.unwrap();
}

async fn helius_webhook_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<HeliusWebhook>,
) -> impl IntoResponse {
    println!("Received webhook: {:?}", payload);

    // Extract transaction signature
    let signature = match &payload.transaction {
        Some(tx) => tx.signature.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(WebhookResponse {
                    success: false,
                    message: "No transaction data".to_string(),
                }),
            );
        }
    };

    // Process native transfers
    if let Some(transfers) = payload.native_transfers {
        for transfer in transfers {
            let amount_lamports = transfer.amount as i64;
            let amount_sol = amount_lamports as f64 / 1_000_000_000.0;
            
            let timestamp = DateTime::from_timestamp(payload.timestamp, 0)
                .unwrap_or_else(|| Utc::now());

            // Insert into database
            match sqlx::query(
                r#"
                INSERT INTO transactions (signature, sender, receiver, amount_lamports, amount_sol, timestamp)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (signature) DO NOTHING
                "#
            )
            .bind(&signature)
            .bind(&transfer.from_user_account)
            .bind(&transfer.to_user_account)
            .bind(amount_lamports)
            .bind(amount_sol)
            .bind(timestamp)
            .execute(&state.db)
            .await
            {
                Ok(_) => {
                    println!(
                        "Stored transaction: {} SOL from {} to {}",
                        amount_sol,
                        transfer.from_user_account,
                        transfer.to_user_account
                    );
                }
                Err(e) => {
                    eprintln!("Database error: {:?}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(WebhookResponse {
                            success: false,
                            message: format!("Database error: {}", e),
                        }),
                    );
                }
            }
        }
    }

    (
        StatusCode::OK,
        Json(WebhookResponse {
            success: true,
            message: "Webhook processed successfully".to_string(),
        }),
    )
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}