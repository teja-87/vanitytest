use axum::{extract::State, Json, Router, routing::post};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing_subscriber::fmt::format;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use reqwest::Client;


use ed25519_dalek::{Verifier, VerifyingKey, Signature as DalekSignature};
use bs58;


#[derive(Clone)]
struct AppState {
    pool: Arc<PgPool>,
    worker_client:Client,
    worker_url: String,
}

#[tokio::main]
async fn main() {
  
    let db_url = "postgresql://neondb_owner:npg_fxOyd0mlEh4e@ep-dawn-math-a19l7rm1.ap-southeast-1.aws.neon.tech/neondb?sslmode=require&channel_binding=require";

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
    let worker_url="http://127.0.0.1:5000".to_string();
    let client=Client::new();
    let state = AppState { 
        pool: Arc::new(pool) ,

        worker_client:client,
        worker_url:worker_url,
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
        message:Vec<u8>,
        
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]

struct JobPayLoad{
    word:String,
}
async fn checkdata(State(state): State<AppState>, Json(data): Json<FrontDa>) -> Json<Value> {
    println!("front data{:?} ", data);
   
    
    let message = data.message;
    let sign = data.sign;
    let publickey =data.publickey;
    let word=data.word;

    match verify_sig(&message,&sign,&publickey).await{

        Ok(_)=>{
             println!("✅ SIGNATURE VERIFIED! User owns this wallet!");
             println!("========================================\n");
             let ret =check_db( &state.pool,&publickey).await;
             match ret{
                Ok(r)=>{
                    println!("DEBUG → is_paid={}, is_used={}", r.is_paid, r.is_used);

                    if r.is_paid==true && r.is_used==false{
                            match isused(&state.pool, &r.tx_signature).await {
                                            Ok(_) => println!("✅ Marked as used"),  
                                            Err(e) => println!("❌ Failed to mark as used: {}", e),
                                        }
                            
                            
                        // vanity generate code function goes here
                        let payload = JobPayLoad{
                            word:word.clone(),
                        };

                        let worker_url = format!("{}/wordyword", state.worker_url);

                        let han=tokio::spawn(async move{
                             state.worker_client.post(&worker_url).json(&payload)
                            .send().await});
                               
                             match han.await {
                                    Ok(Ok(response)) => {
                                        match response.json::<serde_json::Value>().await {
                                            Ok(worker_data) => {
                                                println!("🔥 WORKER DATA: {:?}", worker_data);
                                               return  Json(json!({
                                                    "data":worker_data
                                                }))
                                            }
                                            Err(e) => {
                                                eprintln!("❌ Failed to parse worker response JSON: {}", e);
                                            }
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        eprintln!("❌ HTTP request to worker failed: {}", e);
                                    }
                                    Err(e) => {
                                        eprintln!("❌ Worker task panicked and failed: {}", e);
                                    }
                                }
                                                    


                    }
                    else if r.is_paid==true && r.is_used==true {

                            let msg= format!("{},is already used ",&publickey);
                            println!("returning the used true data ");
                        return Json(
                            json!({"msg":msg}),
                        );

                    }
                }
                Err(e)=>{
                     println!("error checking data:{}",e);
                }
             }
            
               Json(json!({
                "status": "success",
                "message": "Signature verified successfully!",
                "verified": true
            }))
        }
        Err(e)=> {
             println!("❌ VERIFICATION FAILED: {}", e);
          
            Json(json!({
                "status": "error",
                "message": format!("Signature verification failed: {}", e),
                "verified": false
            }))
            }

    
  
}
}

async fn verify_sig(message:&[u8],sign:&[u8],publickey:&str)->Result<(),String>{


     
    let signe = DalekSignature::from_bytes(sign.try_into().map_err(|e| format!("invalid sign:{}",e))?);
    let publikey =bs58::decode(&publickey).into_vec().map_err(|e| format!("Invalid msg type {:?}",e))?;;
    let verifying_key=VerifyingKey::from_bytes(publikey.as_slice().try_into().map_err(|_| "Failed to parse public key")?
    ).map_err(|e| format!("Invalid verifying key: {}", e))?;


    verifying_key.verify(message, &signe).map_err(|e| format!("Verification failed: {}", e))?;


      Ok(())
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
    _slot: i64,
    timestamp: Option<i64>,
    receiver: &str,
) -> Result<(), sqlx::Error> {
    let amount_sol = lamports as f64 / 1_000_000_000.0;
    
    // Check if payment is sufficient
    let is_paid = amount_sol >= 0.1;
    
    // Use provided timestamp or current time
    let paid_at = timestamp.map(|ts| {
        chrono::DateTime::from_timestamp(ts, 0)
            .unwrap_or_else(|| chrono::Utc::now())
    }).unwrap_or_else(|| chrono::Utc::now());
    
    sqlx::query(
        r#"
        INSERT INTO vanity_orders 
        (tx_signature, sender, receiver, amount_sol, paid_at, is_paid)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (tx_signature) DO NOTHING
        "#
    )
    .bind(signature)
    .bind(sender)
    .bind(receiver)
    .bind(amount_sol)
    .bind(paid_at)
    .bind(is_paid)
    .execute(pool)
    .await?;
    
    Ok(())
}

//function for checing the payment
#[derive(Debug,sqlx::FromRow)]
struct Rowsql{
    is_paid:bool,
    amount_sol:f64,
    is_used:bool,
    is_generated:bool,
    tx_signature:String,
}

async fn check_db(pool: &PgPool,publickey:&str)->Result<Rowsql,String>{

    println!("checking db here and this is the publickey : {}", publickey);
         
        let row= sqlx::query_as::<_, Rowsql>(
            r#"SELECT is_paid, amount_sol,is_used,is_generated, tx_signature from vanity_orders WHERE sender=$1"#)
         .bind(publickey).fetch_one(pool).await;

        match row{
            Ok(r)=>{
                println!("the sql row is :{:?}", r);
                return Ok(r)
            }

            Err(e)=>{
                println!("there is a error:{}",e);
                return Err(e.to_string())
            }
        }
        
       
}

async fn isused(pool: &PgPool,signatur:&str)->Result<sqlx::postgres::PgQueryResult,String>{
    let insert=sqlx::query(r#"UPDATE vanity_orders SET is_used=TRUE WHERE tx_signature=$1 "#)
    .bind(signatur).execute(pool).await;

        match insert {
            Ok(i)=>{
                Ok(i)
            }
            Err(e)=>{
                Err(e.to_string())
            }
            
        }
}