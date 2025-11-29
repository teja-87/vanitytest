use axum::{extract::State, Json, Router, routing::post};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
struct AppState {
    pool: Arc<PgPool>,
}

#[tokio::main]
async fn main() {
  
    let db_url = "";

    println!("🔌 Connecting to database...");
    println!("📍 Host: {}", db_url.split('@').nth(1).unwrap_or("hidden").split('/').next().unwrap_or("hidden"));
    
    let pool = match PgPool::connect(&db_url).await {
        Ok(p) => {
            println!("✅ Database connected successfully!");
            p
        }
        Err(e) => {
            eprintln!("❌ Failed to connect: {:?}", e);
            eprintln!("\n💡 Tips:");
            eprintln!("1. Check your DATABASE_URL is correct");
            eprintln!("2. Ensure your database is not paused");
            eprintln!("3. Test with: psql \"YOUR_CONNECTION_STRING\"");
            eprintln!("4. Try different provider (Neon/Supabase/Nhost)");
            panic!("Cannot start without database");
        }
    };

    // Test database with simple query
    match sqlx::query("SELECT 1 as test").fetch_one(&pool).await {
        Ok(_) => println!("✅ Database query test passed!"),
        Err(e) => {
            eprintln!("❌ Database query failed: {:?}", e);
            eprintln!("Connection works but queries are failing - check your database!");
            panic!("Database issue");
        }
    }
    
    println!("\n🚀 Server starting on 0.0.0.0:3000");
    
    let state = AppState { 
        pool: Arc::new(pool) 
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
   
    let app = Router::new()
        .route("/webhook", post(webhook_handler))
        .route("/frontdata", post(checkdata))
        .route("/health", axum::routing::get(health_check))
        .layer(cors)
        .with_state(state);
    
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    
    println!("👂 Listening on http://0.0.0.0:3000");
    println!("📡 Webhook endpoint: http://0.0.0.0:3000/webhook");
    println!("🏥 Health check: http://0.0.0.0:3000/health\n");
    
    axum::serve(listener, app).await.unwrap();
}

async fn webhook_handler(
    State(state): State<AppState>,
    Json(thedata): Json<Value>,
) -> Json<Value> {
    println!("\n🔔 ========== WEBHOOK RECEIVED ==========");
    println!("📦 Full data: {:#}", thedata);
    
    // Helius sends an array of transactions
    let transactions = if thedata.is_array() {
        thedata.as_array().unwrap()
    } else {
        // Fallback: maybe it's a single object
        println!("⚠️  Data is not an array, treating as single transaction");
        return Json(json!({
            "status": "error",
            "message": "Expected array of transactions"
        }));
    }; 
    
    println!("📋 Found {} transaction(s)", transactions.len());
    
    for (idx, tx) in transactions.iter().enumerate() {
        println!("\n📌 Transaction #{}", idx + 1);
        
        let sig = tx["signature"].as_str().unwrap_or("UNKNOWN");
        let slot = tx["slot"].as_i64().unwrap_or(0);
        let timestamp = tx["timestamp"].as_i64();
        let fee_payer = tx["feePayer"].as_str().unwrap_or("UNKNOWN");
        
        println!("   🔑 Signature: {}", sig);
        println!("   📊 Slot: {}", slot);
        println!("   ⏰ Timestamp: {}", timestamp.unwrap_or(0));
        println!("   💳 Fee Payer: {}", fee_payer);
        
        // Process native transfers
        if let Some(native_transfers) = tx["nativeTransfers"].as_array() {
            println!("   💸 Native Transfers: {}", native_transfers.len());
            
            for transfer in native_transfers {
                let from = transfer["fromUserAccount"].as_str().unwrap_or("UNKNOWN");
                let to = transfer["toUserAccount"].as_str().unwrap_or("UNKNOWN");
                let amount = transfer["amount"].as_u64().unwrap_or(0);
                let amount_sol = amount as f64 / 1_000_000_000.0;
                
                println!("\n   💰 Transfer:");
                println!("      📤 From: {}", from);
                println!("      📥 To: {}", to);
                println!("      💵 Amount: {} lamports ({} SOL)", amount, amount_sol);
                
                // Insert into database
                match add_paid(
                    &state.pool,
                    sig,
                    from,
                    amount,
                    slot,
                    timestamp,
                    to
                ).await {
                    Ok(_) => println!("      ✅ DB INSERT SUCCESS"),
                    Err(e) => println!("      ❌ DB INSERT FAILED: {:?}", e),
                }
            }
        } else {
            println!("   ⚠️  No native transfers found");
        }
    }
    
    println!("========================================\n");
    
    // Respond fast to Helius
    Json(json!({
        "status": "ok bro",
        "received": true
    }))
}

#[derive(Debug, Deserialize)]
struct FrontDa {
        word:String,
        publickey:String,
        sign:Vec<u8>,
        
}
async fn checkdata(State(_state): State<AppState>, Json(data): Json<FrontDa>) -> Json<Value> {
    println!("front data{:?}", data);
    println!("front data{:?}", data.publickey);
    println!("front data{:?}", data.sign);
    println!("front data{:?}", data.word);
    
    



    
    



    Json(json!({"status": "ok"}))
}

async fn health_check() -> &'static str {
    "OK"
}

// Database function - add transaction
async fn add_paid(
    pool: &PgPool,
    signature: &str,
    sender: &str,
    lamports: u64,
    _slot: i64, // Keep parameter but don't use it
    timestamp: Option<i64>,
    receiver: &str,
) -> Result<(), sqlx::Error> {
    let amount_sol = lamports as f64 / 1_000_000_000.0;
    
    // Use provided timestamp or current time
    let ts = timestamp.unwrap_or_else(|| chrono::Utc::now().timestamp());
    
    sqlx::query(
        r#"
        INSERT INTO transactions (signature, sender, receiver, amount_lamports, amount_sol, timestamp)
        VALUES ($1, $2, $3, $4, $5, to_timestamp($6))
        ON CONFLICT (signature) DO NOTHING
        "#
    )
    .bind(signature)
    .bind(sender)
    .bind(receiver)
    .bind(lamports as i64)
    .bind(amount_sol)
    .bind(ts)
    .execute(pool)
    .await?;
    
    Ok(())
}