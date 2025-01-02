use anyhow::Result;
use async_trait::async_trait;
use mcp_sdk::{
    transport::{Transport, JsonRpcMessage},
    types::{CallToolRequest, CallToolResponse, Tool, ToolResponseContent},
};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
};
use std::str::FromStr;

struct SolanaMcpServer {
    rpc_client: RpcClient,
}

impl SolanaMcpServer {
    fn new() -> Self {
        let rpc_url = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| String::from("https://api.mainnet-beta.solana.com"));
        
        Self {
            rpc_client: RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed()),
        }
    }

    async fn get_balance(&self, pubkey: &Pubkey) -> Result<u64> {
        Ok(self.rpc_client.get_balance(pubkey)?)
    }

    async fn get_token_balance(&self, token_account: &Pubkey) -> Result<u64> {
        let account = self.rpc_client.get_token_account(token_account)?;
        Ok(account.map(|acc| acc.token_amount.amount.parse().unwrap_or(0)).unwrap_or(0))
    }

    async fn get_slot(&self) -> Result<u64> {
        Ok(self.rpc_client.get_slot()?)
    }

    async fn get_block_time(&self, slot: u64) -> Result<i64> {
        Ok(self.rpc_client.get_block_time(slot)?)
    }

    async fn get_transaction(&self, signature: &str) -> Result<String> {
        let sig = signature.parse()
            .map_err(|_| anyhow::anyhow!("Invalid transaction signature"))?;
        let tx = self.rpc_client.get_transaction(&sig, None)?;
        Ok(serde_json::to_string(&tx)?)
    }

    async fn get_block(&self, slot: u64) -> Result<String> {
        let block = self.rpc_client.get_block(slot)?;
        Ok(serde_json::to_string(&block)?)
    }

    async fn get_epoch_info(&self) -> Result<String> {
        let info = self.rpc_client.get_epoch_info()?;
        Ok(serde_json::to_string(&info)?)
    }

    async fn get_account_info(&self, pubkey: &Pubkey) -> Result<String> {
        let account = self.rpc_client.get_account(pubkey)?;
        Ok(serde_json::to_string(&account)?)
    }

    async fn get_program_accounts(&self, program_id: &Pubkey) -> Result<String> {
        let accounts = self.rpc_client.get_program_accounts(program_id)?;
        Ok(serde_json::to_string(&accounts)?)
    }

    async fn get_recent_blockhash(&self) -> Result<String> {
        let (blockhash, fee_calculator) = self.rpc_client.get_recent_blockhash()?;
        Ok(serde_json::json!({
            "blockhash": blockhash.to_string(),
            "fee_calculator": fee_calculator
        }).to_string())
    }

    async fn get_version(&self) -> Result<String> {
        let version = self.rpc_client.get_version()?;
        Ok(serde_json::to_string(&version)?)
    }

    async fn get_health(&self) -> Result<String> {
        match self.rpc_client.get_health() {
            Ok(_) => Ok("ok".to_string()),
            Err(e) => Ok(format!("error: {}", e))
        }
    }

    async fn get_minimum_balance_for_rent_exemption(&self, data_size: usize) -> Result<u64> {
        Ok(self.rpc_client.get_minimum_balance_for_rent_exemption(data_size)?)
    }

    async fn get_supply(&self) -> Result<String> {
        let supply = self.rpc_client.get_supply()?;
        Ok(serde_json::to_string(&supply)?)
    }

    async fn get_largest_accounts(&self) -> Result<String> {
        let accounts = self.rpc_client.get_largest_accounts()?;
        Ok(serde_json::to_string(&accounts)?)
    }

    async fn get_inflation_rate(&self) -> Result<String> {
        let rate = self.rpc_client.get_inflation_rate()?;
        Ok(serde_json::to_string(&rate)?)
    }

    async fn get_cluster_nodes(&self) -> Result<String> {
        let nodes = self.rpc_client.get_cluster_nodes()?;
        Ok(serde_json::to_string(&nodes)?)
    }

    async fn get_token_accounts_by_owner(&self, owner: &Pubkey) -> Result<String> {
        let accounts = self.rpc_client.get_token_accounts_by_owner(
            owner,
            solana_client::rpc_config::RpcTokenAccountsFilter::ProgramId(spl_token::id()),
        )?;
        Ok(serde_json::to_string(&accounts)?)
    }

    async fn get_token_accounts_by_delegate(&self, delegate: &Pubkey) -> Result<String> {
        let accounts = self.rpc_client.get_token_accounts_by_delegate(
            delegate,
            solana_client::rpc_config::RpcTokenAccountsFilter::ProgramId(spl_token::id()),
        )?;
        Ok(serde_json::to_string(&accounts)?)
    }

    async fn get_token_supply(&self, mint: &Pubkey) -> Result<String> {
        let supply = self.rpc_client.get_token_supply(mint)?;
        Ok(serde_json::to_string(&supply)?)
    }

    async fn get_vote_accounts(&self) -> Result<String> {
        let accounts = self.rpc_client.get_vote_accounts()?;
        Ok(serde_json::to_string(&accounts)?)
    }

    async fn get_leader_schedule(&self) -> Result<String> {
        let schedule = self.rpc_client.get_leader_schedule(None)?;
        Ok(serde_json::to_string(&schedule)?)
    }

    fn get_tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: "get_sol_balance".to_string(),
                description: Some("Get SOL balance for a Solana address".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "address": {
                            "type": "string",
                            "description": "Solana wallet address"
                        }
                    },
                    "required": ["address"]
                }),
            },
            Tool {
                name: "get_token_balance".to_string(),
                description: Some("Get SPL token balance for a token account".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "token_account": {
                            "type": "string",
                            "description": "Token account address"
                        }
                    },
                    "required": ["token_account"]
                }),
            },
            Tool {
                name: "get_slot".to_string(),
                description: Some("Get current slot".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_block_time".to_string(),
                description: Some("Get estimated production time of a block".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "slot": {
                            "type": "number",
                            "description": "Block slot number"
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
                            "type": "number",
                            "description": "Block slot number"
                        }
                    },
                    "required": ["slot"]
                }),
            },
            Tool {
                name: "get_epoch_info".to_string(),
                description: Some("Get information about the current epoch".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
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
                            "description": "Account address"
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
                description: Some("Get recent blockhash and fee calculator".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_version".to_string(),
                description: Some("Get Solana node version information".to_string()),
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
                            "type": "number",
                            "description": "Size of account data in bytes"
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
                description: Some("Get the largest accounts on the network".to_string()),
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
                description: Some("Get information about all nodes in the cluster".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_token_accounts_by_owner".to_string(),
                description: Some("Get all token accounts owned by an address".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {
                            "type": "string",
                            "description": "Owner's wallet address"
                        }
                    },
                    "required": ["owner"]
                }),
            },
            Tool {
                name: "get_token_accounts_by_delegate".to_string(),
                description: Some("Get all token accounts with delegation to an address".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "delegate": {
                            "type": "string",
                            "description": "Delegate's wallet address"
                        }
                    },
                    "required": ["delegate"]
                }),
            },
            Tool {
                name: "get_token_supply".to_string(),
                description: Some("Get total supply of a token".to_string()),
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
                description: Some("Get all vote accounts".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            Tool {
                name: "get_leader_schedule".to_string(),
                description: Some("Get leader schedule for current epoch".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        ]
    }

    async fn handle_request(&self, request: CallToolRequest) -> Result<CallToolResponse> {
        match request.name.as_str() {
            "get_slot" => {
                let slot = self.get_slot().await?;
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
                
                let timestamp = self.get_block_time(slot).await?;
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
                
                let tx_info = self.get_transaction(signature).await?;
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
                
                let block_info = self.get_block(slot).await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Block info: {}", block_info),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_epoch_info" => {
                let info = self.get_epoch_info().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Epoch info: {}", info),
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
                
                let info = self.get_account_info(&pubkey).await?;
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
                
                let accounts = self.get_program_accounts(&pubkey).await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Program accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_recent_blockhash" => {
                let info = self.get_recent_blockhash().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Recent blockhash info: {}", info),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_version" => {
                let version = self.get_version().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Version info: {}", version),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_health" => {
                let health = self.get_health().await?;
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
                
                let balance = self.get_minimum_balance_for_rent_exemption(data_size).await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Minimum balance for rent exemption: {} lamports", balance),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_supply" => {
                let supply = self.get_supply().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Supply info: {}", supply),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_largest_accounts" => {
                let accounts = self.get_largest_accounts().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Largest accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_inflation_rate" => {
                let rate = self.get_inflation_rate().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Inflation rate: {}", rate),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_cluster_nodes" => {
                let nodes = self.get_cluster_nodes().await?;
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
                
                let accounts = self.get_token_accounts_by_owner(&pubkey).await?;
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
                
                let accounts = self.get_token_accounts_by_delegate(&pubkey).await?;
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
                
                let supply = self.get_token_supply(&pubkey).await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Token supply: {}", supply),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_vote_accounts" => {
                let accounts = self.get_vote_accounts().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Vote accounts: {}", accounts),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_leader_schedule" => {
                let schedule = self.get_leader_schedule().await?;
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Leader schedule: {}", schedule),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_sol_balance" => {
                let address = request.arguments.as_ref()
                    .and_then(|args| args.get("address"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing address parameter"))?;
                
                let pubkey = Pubkey::from_str(address)
                    .map_err(|_| anyhow::anyhow!("Invalid Solana address"))?;
                
                let balance = self.get_balance(&pubkey).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Balance: {} SOL", balance as f64 / 1_000_000_000.0),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            "get_token_balance" => {
                let token_account = request.arguments.as_ref()
                    .and_then(|args| args.get("token_account"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing token_account parameter"))?;
                
                let account_pubkey = Pubkey::from_str(token_account)
                    .map_err(|_| anyhow::anyhow!("Invalid token account address"))?;
                
                let balance = self.get_token_balance(&account_pubkey).await?;
                
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!("Token Balance: {}", balance),
                    }],
                    is_error: Some(false),
                    meta: None,
                })
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", request.name)),
        }
    }
}

#[async_trait]
impl Transport for SolanaMcpServer {
    fn send<'a>(&'a self, message: &'a JsonRpcMessage) -> Result<()> {
        match message {
            JsonRpcMessage::Request(request) => {
                if request.method == "list_tools" {
                    let tools = self.get_tools();
                    let response = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request.id,
                        "result": tools,
                    });
                    println!("{}", serde_json::to_string(&response)?);
                } else if request.method == "call_tool" {
                    if let Some(params) = &request.params {
                        let tool_request: CallToolRequest = serde_json::from_value(params.clone())?;
                        let tool_response = tokio::runtime::Runtime::new()?.block_on(self.handle_request(tool_request))?;
                        let response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": request.id,
                            "result": tool_response,
                        });
                        println!("{}", serde_json::to_string(&response)?);
                    }
                }
            }
            _ => println!("{}", serde_json::to_string(message)?),
        }
        Ok(())
    }

    fn receive<'a>(&'a self) -> Result<JsonRpcMessage> {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        Ok(serde_json::from_str(&input)?)
    }

    fn open(&self) -> Result<()> {
        Ok(())
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = SolanaMcpServer::new();
    
    println!("Solana MCP server running on stdio");
    
    loop {
        let message = server.receive()?;
        server.send(&message)?;
    }
}
