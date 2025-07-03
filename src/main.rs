use anyhow::Result;
use log::{error, info, warn};
use mcp_sdk::types::{
    CallToolRequest, CallToolResponse, Tool, ToolResponseContent,
};
use solana_client::rpc_client::RpcClient;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
};
use solana_transaction_status::UiTransactionEncoding;
use spl_token;
use std::{str::FromStr, time::{Duration, Instant}};
use tokio::time::{timeout, sleep};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REQUESTS_PER_MINUTE: u32 = 60;
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: String, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message, data }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceTemplate {
    pub uri_template: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

struct CircuitBreaker {
    failures: std::sync::atomic::AtomicU32,
    last_failure: std::sync::Mutex<Instant>,
    threshold: u32,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    fn new(threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            failures: std::sync::atomic::AtomicU32::new(0),
            last_failure: std::sync::Mutex::new(Instant::now()),
            threshold,
            reset_timeout,
        }
    }

    fn record_failure(&self) -> bool {
        let mut last_failure = self.last_failure.lock().unwrap();
        let current_time = Instant::now();
        
        if current_time.duration_since(*last_failure) > self.reset_timeout {
            self.failures.store(0, std::sync::atomic::Ordering::SeqCst);
        }
        
        *last_failure = current_time;
        let failures = self.failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        failures >= self.threshold
    }

    fn is_open(&self) -> bool {
        let failures = self.failures.load(std::sync::atomic::Ordering::SeqCst);
        let last_failure = *self.last_failure.lock().unwrap();
        
        failures >= self.threshold && 
        Instant::now().duration_since(last_failure) <= self.reset_timeout
    }

    fn reset(&self) {
        self.failures.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

struct SolanaMcpServer {
    rpc_client: RpcClient,
    request_count: std::sync::atomic::AtomicU32,
    last_reset: std::sync::Mutex<Instant>,
    circuit_breaker: CircuitBreaker,
}

impl SolanaMcpServer {
    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        Ok(vec![
            Tool {
                name: "get_sol_balance".to_string(),
                description: Some("Get SOL balance for an address".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "address": {
                            "type": "string",
                            "description": "Account public key"
                        }
                    },
                    "required": ["address"]
                }),
            },
            Tool {
                name: "get_token_balance".to_string(),
                description: Some("Get SPL token balance".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "address": {
                            "type": "string",
                            "description": "Token account public key"
                        }
                    },
                    "required": ["address"]
                }),
            },
            Tool {
                name: "get_epoch_info".to_string(),
                description: Some("Get current epoch information".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_slot".to_string(),
                description: Some("Get the current slot".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_block_time".to_string(),
                description: Some("Get the estimated production time of a block".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "slot": {
                            "type": "integer",
                            "description": "Slot number"
                        }
                    },
                    "required": ["slot"]
                }),
            },
            Tool {
                name: "get_transaction".to_string(),
                description: Some("Get transaction details".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "signature": {
                            "type": "string",
                            "description": "Transaction signature"
                        }
                    },
                    "required": ["signature"]
                }),
            },
            Tool {
                name: "get_block".to_string(),
                description: Some("Get block information".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "slot": {
                            "type": "integer",
                            "description": "Slot number"
                        }
                    },
                    "required": ["slot"]
                }),
            },
            Tool {
                name: "get_account_info".to_string(),
                description: Some("Get account information".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "address": {
                            "type": "string",
                            "description": "Account public key"
                        }
                    },
                    "required": ["address"]
                }),
            },
            Tool {
                name: "get_program_accounts".to_string(),
                description: Some("Get all accounts owned by a program".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "program_id": {
                            "type": "string",
                            "description": "Program ID (public key)"
                        }
                    },
                    "required": ["program_id"]
                }),
            },
            Tool {
                name: "get_recent_blockhash".to_string(),
                description: Some("Get a recent block hash".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_version".to_string(),
                description: Some("Get Solana version information".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_health".to_string(),
                description: Some("Get node health status".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_minimum_balance_for_rent_exemption".to_string(),
                description: Some("Get minimum balance required for rent exemption".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "data_size": {
                            "type": "integer",
                            "description": "Size of account data"
                        }
                    },
                    "required": ["data_size"]
                }),
            },
            Tool {
                name: "get_supply".to_string(),
                description: Some("Get information about the current supply".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_largest_accounts".to_string(),
                description: Some("Get the largest accounts".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_inflation_rate".to_string(),
                description: Some("Get the current inflation rate".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_cluster_nodes".to_string(),
                description: Some("Get information about all the nodes participating in the cluster".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_token_accounts_by_owner".to_string(),
                description: Some("Get all token accounts owned by the specified account".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {
                            "type": "string",
                            "description": "Owner's public key"
                        }
                    },
                    "required": ["owner"]
                }),
            },
            Tool {
                name: "get_token_accounts_by_delegate".to_string(),
                description: Some("Get all token accounts that delegate authority to the specified account".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "delegate": {
                            "type": "string",
                            "description": "Delegate's public key"
                        }
                    },
                    "required": ["delegate"]
                }),
            },
            Tool {
                name: "get_token_supply".to_string(),
                description: Some("Get token supply information".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "mint": {
                            "type": "string",
                            "description": "Token mint address"
                        }
                    },
                    "required": ["mint"]
                }),
            },
            Tool {
                name: "get_vote_accounts".to_string(),
                description: Some("Get information about all vote accounts".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_leader_schedule".to_string(),
                description: Some("Get the leader schedule for the current epoch".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        ])
    }

    fn new() -> Self {
        let circuit_breaker = CircuitBreaker::new(5, Duration::from_secs(60));
        let rpc_url = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| String::from("https://api.mainnet-beta.solana.com"));
        
        info!("Initializing Solana MCP server with RPC URL: {}", rpc_url);
        
        Self {
            rpc_client: RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed()),
            request_count: std::sync::atomic::AtomicU32::new(0),
            last_reset: std::sync::Mutex::new(Instant::now()),
            circuit_breaker,
        }
    }

    async fn retry_with_backoff<F, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Result<T>,
    {
        let mut retries = 0;
        loop {
            match operation() {
                Ok(result) => {
                    self.circuit_breaker.reset();
                    return Ok(result);
                }
                Err(e) => {
                    retries += 1;
                    if retries >= MAX_RETRIES {
                        if self.circuit_breaker.record_failure() {
                            error!("Circuit breaker opened due to too many failures");
                        }
                        return Err(e);
                    }
                    warn!("Operation failed, retrying in {:?}: {}", RETRY_DELAY, e);
                    sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    fn check_rate_limit(&self) -> Result<()> {
        let mut last_reset = self.last_reset.lock().unwrap();
        let current_time = Instant::now();
        let elapsed = current_time.duration_since(*last_reset);

        if elapsed >= Duration::from_secs(60) {
            self.request_count.store(0, std::sync::atomic::Ordering::SeqCst);
            *last_reset = current_time;
        }

        let count = self.request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count >= MAX_REQUESTS_PER_MINUTE {
            warn!("Rate limit exceeded: {} requests in the last minute", count);
            return Err(anyhow::anyhow!("Rate limit exceeded"));
        }

        Ok(())
    }

    pub async fn handle_tool_request(&self, request: CallToolRequest) -> Result<CallToolResponse> {
        if self.circuit_breaker.is_open() {
            return Err(anyhow::anyhow!("Service temporarily unavailable"));
        }

        // Check rate limit first
        self.check_rate_limit()?;

        let tool_name = request.name.clone();
        let start_time = Instant::now();
        let result = timeout(REQUEST_TIMEOUT, self.process_tool_request(request)).await;
        let duration = start_time.elapsed();

        match result {
            Ok(response) => {
                info!("Tool {} executed successfully in {:?}", tool_name, duration);
                response
            }
            Err(_) => {
                error!("Tool {} timed out after {:?}", tool_name, duration);
                if self.circuit_breaker.record_failure() {
                    error!("Circuit breaker opened due to timeout");
                }
                Err(anyhow::anyhow!("Request timed out"))
            }
        }
    }

    async fn process_tool_request(&self, request: CallToolRequest) -> Result<CallToolResponse> {
        match request.name.as_str() {
            "get_sol_balance" => {
                let address = request.arguments.as_ref()
                    .and_then(|args| args.get("address"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing address parameter"))?;
                
                let pubkey = Pubkey::from_str(address)
                    .map_err(|_| anyhow::anyhow!("Invalid account address"))?;
                
                let balance = self.retry_with_backoff(|| {
                    self.rpc_client.get_balance(&pubkey)
                        .map_err(|e| anyhow::anyhow!("Failed to get balance: {}", e))
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("SOL balance: {} lamports ({:.9} SOL)", balance, balance as f64 / 1_000_000_000.0),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_token_balance" => {
                let address = request.arguments.as_ref()
                    .and_then(|args| args.get("address"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing address parameter"))?;
                
                let pubkey = Pubkey::from_str(address)
                    .map_err(|_| anyhow::anyhow!("Invalid token account address"))?;
                
                let balance = self.retry_with_backoff(|| {
                    let balance = self.rpc_client.get_token_account_balance(&pubkey)
                        .map_err(|e| anyhow::anyhow!("Failed to get token balance: {}", e))?;
                    Ok(serde_json::to_string(&balance)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Token balance: {}", balance),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_epoch_info" => {
                let epoch_info = self.retry_with_backoff(|| {
                    let epoch = self.rpc_client.get_epoch_info()
                        .map_err(|e| anyhow::anyhow!("Failed to get epoch info: {}", e))?;
                    Ok(serde_json::to_string(&epoch)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Epoch info: {}", epoch_info),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_slot" => {
                let slot = self.retry_with_backoff(|| {
                    self.rpc_client.get_slot()
                        .map_err(|e| anyhow::anyhow!("Failed to get slot: {}", e))
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Current slot: {}", slot),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_block_time" => {
                let slot = request.arguments.as_ref()
                    .and_then(|args| args.get("slot"))
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("Missing or invalid slot parameter"))?;
                
                let timestamp = self.retry_with_backoff(|| {
                    self.rpc_client.get_block_time(slot)
                        .map_err(|e| anyhow::anyhow!("Failed to get block time: {}", e))
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Block time: {:?} (Unix timestamp)", timestamp),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_transaction" => {
                let signature = request.arguments.as_ref()
                    .and_then(|args| args.get("signature"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing signature parameter"))?;
                
                let tx_info = self.retry_with_backoff(|| {
                    let sig = signature.parse()
                        .map_err(|_| anyhow::anyhow!("Invalid transaction signature"))?;
                    let tx = self.rpc_client.get_transaction(&sig, UiTransactionEncoding::Json)
                        .map_err(|e| anyhow::anyhow!("Failed to get transaction: {}", e))?;
                    Ok(serde_json::to_string(&tx)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Transaction info: {}", tx_info),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_block" => {
                let slot = request.arguments.as_ref()
                    .and_then(|args| args.get("slot"))
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("Missing or invalid slot parameter"))?;
                
                let block_info = self.retry_with_backoff(|| {
                    let block = self.rpc_client.get_block(slot)
                        .map_err(|e| anyhow::anyhow!("Failed to get block: {}", e))?;
                    Ok(serde_json::to_string(&block)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Block info: {}", block_info),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_account_info" => {
                let address = request.arguments.as_ref()
                    .and_then(|args| args.get("address"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing address parameter"))?;
                
                let pubkey = Pubkey::from_str(address)
                    .map_err(|_| anyhow::anyhow!("Invalid account address"))?;
                
                let info = self.retry_with_backoff(|| {
                    let account = self.rpc_client.get_account(&pubkey)
                        .map_err(|e| anyhow::anyhow!("Failed to get account: {}", e))?;
                    Ok(serde_json::to_string(&account)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Account info: {}", info),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_program_accounts" => {
                let program_id = request.arguments.as_ref()
                    .and_then(|args| args.get("program_id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing program_id parameter"))?;
                
                let pubkey = Pubkey::from_str(program_id)
                    .map_err(|_| anyhow::anyhow!("Invalid program ID"))?;
                
                let accounts = self.retry_with_backoff(|| {
                    let accounts = self.rpc_client.get_program_accounts(&pubkey)
                        .map_err(|e| anyhow::anyhow!("Failed to get program accounts: {}", e))?;
                    Ok(serde_json::to_string(&accounts)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Program accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_recent_blockhash" => {
                let info = self.retry_with_backoff(|| {
                    let blockhash = self.rpc_client.get_latest_blockhash()
                        .map_err(|e| anyhow::anyhow!("Failed to get latest blockhash: {}", e))?;
                    Ok(serde_json::to_string(&serde_json::json!({
                        "blockhash": blockhash.to_string(),
                    }))?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Recent blockhash info: {}", info),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_version" => {
                let version = self.retry_with_backoff(|| {
                    let version = self.rpc_client.get_version()
                        .map_err(|e| anyhow::anyhow!("Failed to get version: {}", e))?;
                    Ok(serde_json::to_string(&version)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Version info: {}", version),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_health" => {
                let health = self.retry_with_backoff(|| {
                    match self.rpc_client.get_health() {
                        Ok(_) => Ok("ok".to_string()),
                        Err(e) => Ok(format!("error: {}", e))
                    }
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Node health: {}", health),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_minimum_balance_for_rent_exemption" => {
                let data_size = request.arguments.as_ref()
                    .and_then(|args| args.get("data_size"))
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("Missing or invalid data_size parameter"))? as usize;
                
                let balance = self.retry_with_backoff(|| {
                    self.rpc_client.get_minimum_balance_for_rent_exemption(data_size)
                        .map_err(|e| anyhow::anyhow!("Failed to get minimum balance: {}", e))
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Minimum balance for rent exemption: {} lamports", balance),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_supply" => {
                let supply = self.retry_with_backoff(|| {
                    let supply = self.rpc_client.supply_with_commitment(CommitmentConfig::confirmed())
                        .map_err(|e| anyhow::anyhow!("Failed to get supply: {}", e))?;
                    Ok(serde_json::to_string(&supply)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Supply info: {}", supply),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_largest_accounts" => {
                let accounts = self.retry_with_backoff(|| {
                    let accounts = self.rpc_client.get_largest_accounts_with_config(
                        solana_client::rpc_config::RpcLargestAccountsConfig::default()
                    )
                    .map_err(|e| anyhow::anyhow!("Failed to get largest accounts: {}", e))?;
                    Ok(serde_json::to_string(&accounts)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Largest accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_inflation_rate" => {
                let rate = self.retry_with_backoff(|| {
                    let rate = self.rpc_client.get_inflation_rate()
                        .map_err(|e| anyhow::anyhow!("Failed to get inflation rate: {}", e))?;
                    Ok(serde_json::to_string(&rate)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Inflation rate: {}", rate),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_cluster_nodes" => {
                let nodes = self.retry_with_backoff(|| {
                    let nodes = self.rpc_client.get_cluster_nodes()
                        .map_err(|e| anyhow::anyhow!("Failed to get cluster nodes: {}", e))?;
                    Ok(serde_json::to_string(&nodes)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Cluster nodes: {}", nodes),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_token_accounts_by_owner" => {
                let owner = request.arguments.as_ref()
                    .and_then(|args| args.get("owner"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing owner parameter"))?;
                
                let pubkey = Pubkey::from_str(owner)
                    .map_err(|_| anyhow::anyhow!("Invalid owner address"))?;
                
                let accounts = self.retry_with_backoff(|| {
                    let accounts = self.rpc_client.get_token_accounts_by_owner(
                        &pubkey,
                        TokenAccountsFilter::ProgramId(spl_token::id()),
                    )
                    .map_err(|e| anyhow::anyhow!("Failed to get token accounts: {}", e))?;
                    Ok(serde_json::to_string(&accounts)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Token accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_token_accounts_by_delegate" => {
                let delegate = request.arguments.as_ref()
                    .and_then(|args| args.get("delegate"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing delegate parameter"))?;
                
                let pubkey = Pubkey::from_str(delegate)
                    .map_err(|_| anyhow::anyhow!("Invalid delegate address"))?;
                
                let accounts = self.retry_with_backoff(|| {
                    let accounts = self.rpc_client.get_token_accounts_by_delegate(
                        &pubkey,
                        TokenAccountsFilter::ProgramId(spl_token::id()),
                    )
                    .map_err(|e| anyhow::anyhow!("Failed to get delegated token accounts: {}", e))?;
                    Ok(serde_json::to_string(&accounts)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Delegated accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_token_supply" => {
                let mint = request.arguments.as_ref()
                    .and_then(|args| args.get("mint"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing mint parameter"))?;
                
                let pubkey = Pubkey::from_str(mint)
                    .map_err(|_| anyhow::anyhow!("Invalid mint address"))?;
                
                let supply = self.retry_with_backoff(|| {
                    let supply = self.rpc_client.get_token_supply(&pubkey)
                        .map_err(|e| anyhow::anyhow!("Failed to get token supply: {}", e))?;
                    Ok(serde_json::to_string(&supply)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Token supply: {}", supply),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_vote_accounts" => {
                let accounts = self.retry_with_backoff(|| {
                    let accounts = self.rpc_client.get_vote_accounts()
                        .map_err(|e| anyhow::anyhow!("Failed to get vote accounts: {}", e))?;
                    Ok(serde_json::to_string(&accounts)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Vote accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_leader_schedule" => {
                let schedule = self.retry_with_backoff(|| {
                    let schedule = self.rpc_client.get_leader_schedule(None)
                        .map_err(|e| anyhow::anyhow!("Failed to get leader schedule: {}", e))?;
                    Ok(serde_json::to_string(&schedule)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Leader schedule: {}", schedule),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", request.name))
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    
    info!("Starting Solana MCP server");
    let server = SolanaMcpServer::new();
    
    let tools = server.list_tools().await?;
    info!("Available tools:");
    for tool in tools {
        info!("  - {}: {}", tool.name, tool.description.unwrap_or("No description".to_string()));
    }
    
    info!("Solana MCP server initialized with {} tools", 21);
    info!("MCP server listening on stdio...");
    
    // Set up stdio transport for MCP
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();
    
    // Main server loop
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                // EOF reached
                info!("Received EOF, shutting down");
                break;
            }
            Ok(_) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                
                match serde_json::from_str::<JsonRpcRequest>(line) {
                    Ok(request) => {
                        let response = handle_request(&server, request).await;
                        let response_json = serde_json::to_string(&response)?;
                        stdout.write_all(response_json.as_bytes()).await?;
                        stdout.write_all(b"\n").await?;
                        stdout.flush().await?;
                    }
                    Err(e) => {
                        error!("Failed to parse JSON-RPC request: {}", e);
                        let error_response = JsonRpcResponse::error(
                            Value::Null,
                            -32700,
                            "Parse error".to_string(),
                            Some(json!({"error": e.to_string()}))
                        );
                        let response_json = serde_json::to_string(&error_response)?;
                        stdout.write_all(response_json.as_bytes()).await?;
                        stdout.write_all(b"\n").await?;
                        stdout.flush().await?;
                    }
                }
            }
            Err(e) => {
                error!("Failed to read from stdin: {}", e);
                break;
            }
        }
    }
    
    info!("MCP server shutdown");
    Ok(())
}

async fn handle_request(server: &SolanaMcpServer, request: JsonRpcRequest) -> JsonRpcResponse {
    match request.method.as_str() {
        "tools/list" => {
            match server.list_tools().await {
                Ok(tools) => JsonRpcResponse::success(request.id, serde_json::to_value(tools).unwrap()),
                Err(e) => JsonRpcResponse::error(request.id, -32603, e.to_string(), None),
            }
        }
        "tools/call" => {
            match request.params {
                Some(params) => {
                    match serde_json::from_value::<CallToolRequest>(params) {
                        Ok(tool_request) => {
                            match server.handle_tool_request(tool_request).await {
                                Ok(response) => JsonRpcResponse::success(request.id, serde_json::to_value(response).unwrap()),
                                Err(e) => JsonRpcResponse::error(request.id, -32603, e.to_string(), None),
                            }
                        }
                        Err(e) => JsonRpcResponse::error(request.id, -32602, format!("Invalid tool request: {}", e), None),
                    }
                }
                None => JsonRpcResponse::error(request.id, -32602, "Missing parameters".to_string(), None),
            }
        }
        "initialize" => {
            let capabilities = serde_json::json!({
                "tools": {}
            });
            JsonRpcResponse::success(request.id, capabilities)
        }
        _ => JsonRpcResponse::error(request.id, -32601, format!("Method not found: {}", request.method), None),
    }
}
