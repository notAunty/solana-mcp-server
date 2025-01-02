use anyhow::Result;
use async_trait::async_trait;
use log::{error, info, warn};
use mcp_sdk::{
    transport::{Transport, JsonRpcMessage, StdioTransport}, 
    types::{
        CallToolRequest, CallToolResponse, Tool, ToolResponseContent, ErrorCode,
        Resource, ResourceTemplate, ListResourcesRequest, ListResourceTemplatesRequest,
        ReadResourceRequest, ResourceContent, InitializationOptions, ServerCapabilities,
        ResourceCapabilities,
    },
    server::Server,
};
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::TokenAccountsFilter,
};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
};
use solana_transaction_status::UiTransactionEncoding;
use std::{str::FromStr, time::{Duration, Instant}};
use tokio::time::timeout;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::sleep;

// Rest of the code remains the same until retry_with_backoff
// ... (previous code)

impl SolanaMcpServer {
    // ... (previous methods)

    async fn retry_with_backoff<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: Fn() -> Fut + Clone,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut retries = 0;
        loop {
            let f = f.clone();
            match f().await {
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

    // Update resource handlers
    async fn handle_read_resource(&self, request: ReadResourceRequest) -> Result<Vec<ResourceContent>> {
        if self.circuit_breaker.is_open() {
            return Err(anyhow::anyhow!("Service temporarily unavailable"));
        }

        let uri = request.uri.as_str();
        
        // Check cache first
        if let Some(cached) = self.resource_cache.read().await.get(uri) {
            if cached.timestamp.elapsed() < CACHE_DURATION {
                return Ok(cached.content.clone());
            }
        }
        
        // If not in cache or expired, fetch fresh data
        let result = match uri {
            "solana://supply" => {
                let supply = self.retry_with_backoff(|| async {
                    let supply = self.rpc_client.supply()?;
                    Ok(serde_json::to_string(&supply)?)
                }).await?;
                
                let content = vec![ResourceContent {
                    uri: uri.to_string(),
                    text: Some(supply),
                    blob: None,
                    mime_type: Some("application/json".to_string()),
                }];
                self.update_cache(uri, content.clone()).await;
                Ok(content)
            }
            "solana://inflation" => {
                let rate = self.retry_with_backoff(|| async {
                    let rate = self.rpc_client.get_inflation_rate()?;
                    Ok(serde_json::to_string(&rate)?)
                }).await?;
                
                let content = vec![ResourceContent {
                    uri: uri.to_string(),
                    text: Some(rate),
                    blob: None,
                    mime_type: Some("application/json".to_string()),
                }];
                self.update_cache(uri, content.clone()).await;
                Ok(content)
            }
            _ => {
                // Handle dynamic resources
                if uri.starts_with("solana://accounts/") {
                    let address = uri.strip_prefix("solana://accounts/").unwrap();
                    let pubkey = Pubkey::from_str(address)
                        .map_err(|_| anyhow::anyhow!("Invalid account address"))?;
                    
                    let info = self.retry_with_backoff(|| async {
                        let account = self.rpc_client.get_account(&pubkey)?;
                        Ok(serde_json::to_string(&account)?)
                    }).await?;
                    
                    let content = vec![ResourceContent {
                        uri: uri.to_string(),
                        text: Some(info),
                        blob: None,
                        mime_type: Some("application/json".to_string()),
                    }];
                    self.update_cache(uri, content.clone()).await;
                    Ok(content)
                } else if uri.starts_with("solana://transactions/") {
                    let signature = uri.strip_prefix("solana://transactions/").unwrap();
                    
                    let tx_info = self.retry_with_backoff(|| async {
                        let sig = signature.parse()
                            .map_err(|_| anyhow::anyhow!("Invalid transaction signature"))?;
                        let tx = self.rpc_client.get_transaction(&sig, UiTransactionEncoding::Json)?;
                        Ok(serde_json::to_string(&tx)?)
                    }).await?;
                    
                    let content = vec![ResourceContent {
                        uri: uri.to_string(),
                        text: Some(tx_info),
                        blob: None,
                        mime_type: Some("application/json".to_string()),
                    }];
                    self.update_cache(uri, content.clone()).await;
                    Ok(content)
                } else if uri.starts_with("solana://blocks/") {
                    let slot = uri.strip_prefix("solana://blocks/").unwrap()
                        .parse::<u64>()
                        .map_err(|_| anyhow::anyhow!("Invalid slot number"))?;
                    
                    let block_info = self.retry_with_backoff(|| async {
                        let block = self.rpc_client.get_block(slot)?;
                        Ok(serde_json::to_string(&block)?)
                    }).await?;
                    
                    let content = vec![ResourceContent {
                        uri: uri.to_string(),
                        text: Some(block_info),
                        blob: None,
                        mime_type: Some("application/json".to_string()),
                    }];
                    self.update_cache(uri, content.clone()).await;
                    Ok(content)
                } else if uri.starts_with("solana://tokens/") {
                    let parts: Vec<&str> = uri.strip_prefix("solana://tokens/").unwrap().split('/').collect();
                    if parts.len() == 2 && parts[1] == "supply" {
                        let mint = Pubkey::from_str(parts[0])
                            .map_err(|_| anyhow::anyhow!("Invalid mint address"))?;
                        
                        let supply = self.retry_with_backoff(|| async {
                            let supply = self.rpc_client.get_token_supply(&mint)?;
                            Ok(serde_json::to_string(&supply)?)
                        }).await?;
                        
                        let content = vec![ResourceContent {
                            uri: uri.to_string(),
                            text: Some(supply),
                            blob: None,
                            mime_type: Some("application/json".to_string()),
                        }];
                        self.update_cache(uri, content.clone()).await;
                        Ok(content)
                    } else {
                        Err(anyhow::anyhow!("Invalid token resource URI"))
                    }
                } else {
                    Err(anyhow::anyhow!("Resource not found"))
                }
            }
        };

        if result.is_err() && self.circuit_breaker.record_failure() {
            error!("Circuit breaker opened due to too many failures");
        }

        result
    }

    // Update tool handlers
    async fn handle_tool_request(&self, request: CallToolRequest) -> Result<CallToolResponse> {
        match request.name.as_str() {
            "get_slot" => {
                let slot = self.retry_with_backoff(|| async {
                    Ok(self.rpc_client.get_slot()?)
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
                
                let timestamp = self.retry_with_backoff(|| async {
                    Ok(self.rpc_client.get_block_time(slot)?)
                }).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Block time: {} (Unix timestamp)", timestamp),
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
                
                let tx_info = self.retry_with_backoff(|| async {
                    let sig = signature.parse()
                        .map_err(|_| anyhow::anyhow!("Invalid transaction signature"))?;
                    let tx = self.rpc_client.get_transaction(&sig, UiTransactionEncoding::Json)?;
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
                
                let block_info = self.retry_with_backoff(|| async {
                    let block = self.rpc_client.get_block(slot)?;
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
                
                let info = self.retry_with_backoff(|| async {
                    let account = self.rpc_client.get_account(&pubkey)?;
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
                
                let accounts = self.retry_with_backoff(|| async {
                    let accounts = self.rpc_client.get_program_accounts(&pubkey)?;
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
                let info = self.retry_with_backoff(|| async {
                    let (blockhash, fee_calculator) = self.rpc_client.get_latest_blockhash()?;
                    Ok(serde_json::to_string(&serde_json::json!({
                        "blockhash": blockhash.to_string(),
                        "fee_calculator": fee_calculator
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
                let version = self.retry_with_backoff(|| async {
                    let version = self.rpc_client.get_version()?;
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
                let health = self.retry_with_backoff(|| async {
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
                
                let balance = self.retry_with_backoff(|| async {
                    Ok(self.rpc_client.get_minimum_balance_for_rent_exemption(data_size)?)
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
                let supply = self.retry_with_backoff(|| async {
                    let supply = self.rpc_client.supply()?;
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
                let accounts = self.retry_with_backoff(|| async {
                    let accounts = self.rpc_client.get_largest_accounts()?;
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
                let rate = self.retry_with_backoff(|| async {
                    let rate = self.rpc_client.get_inflation_rate()?;
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
                let nodes = self.retry_with_backoff(|| async {
                    let nodes = self.rpc_client.get_cluster_nodes()?;
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
                
                let accounts = self.retry_with_backoff(|| async {
                    let accounts = self.rpc_client.get_token_accounts_by_owner(
                        &pubkey,
                        TokenAccountsFilter::ProgramId(spl_token::id().to_string()),
                    )?;
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
                
                let accounts = self.retry_with_backoff(|| async {
                    let accounts = self.rpc_client.get_token_accounts_by_delegate(
                        &pubkey,
                        TokenAccountsFilter::ProgramId(spl_token::id().to_string()),
                    )?;
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
                
                let supply = self.retry_with_backoff(|| async {
                    let supply = self.rpc_client.get_token_supply(&pubkey)?;
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
                let accounts = self.retry_with_backoff(|| async {
                    let accounts = self.rpc_client.get_vote_accounts()?;
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
                let schedule = self.retry_with_backoff(|| async {
                    let schedule = self.rpc_client.get_leader_schedule(None)?;
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

// Rest of the code remains the same
// ... (Transport implementation and main function)
