use axum::{routing::get, Router, Json, http::StatusCode};
use axum::extract::{Query, State, Path};
use serde::{Serialize, Deserialize};
use std::sync::{Arc, Mutex};
use clap::Parser;

#[derive(Parser)]
struct Args {
  #[arg(long, default_value = "config.toml")]
    config: String,
}

#[derive(Clone)]
struct AppState {
    history_buffer: Arc<Mutex<Vec<HistoryInfo>>>,
    primary_interface: String,
}

#[derive(Deserialize, Clone)]
struct Config {
    listen_address: String,
    poll_interval_seconds: u64,
    primary_interface: String,
    history_retention_seconds: u64,
}

#[derive(Serialize)]
struct InterfaceInfo {
    name: String,
    rx_bytes: u64,
    tx_bytes: u64,
    link_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_primary: Option<bool>,
} 
#[derive(Serialize)]
struct SystemInfo {
    uptime_seconds: f64,
    memory_total_kb: u64,
    memory_used_kb: u64,
    memory_percent: f64,
    cpu_usage_percent: f64,
    cpu_temperature_celsius: Option<f64>,
}

#[derive(Serialize, Clone)]
struct HistoryInfo {
    timestamp: u64,
    uptime_seconds: f64,
    memory_total_kb: u64,
    memory_used_kb: u64,
    memory_percent: f64,
    cpu_usage_percent: f64,
    cpu_temperature_celsius: Option<f64>,
}

#[derive(Deserialize)]
struct HistoryQuery {
    minutes: Option<u64>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let config_text = std::fs::read_to_string(&args.config).expect("Failed to read config file");
    let config: Config = toml::from_str(&config_text).expect("Failed to parse config file");

let history_buffer: Arc<Mutex<Vec<HistoryInfo>>> = Arc::new(Mutex::new(Vec::new()));
let history_for_collector = history_buffer.clone();
let collector_config = config.clone();
tokio::spawn(async move {
    loop{
        tokio::time::sleep(std::time::Duration::from_secs(collector_config.poll_interval_seconds)).await;
        let entry = collect_snapshot().await;
        let mut buffer = history_for_collector.lock().unwrap();
        buffer.push(entry);
        let cutoff = now_timestamp().saturating_sub(collector_config.history_retention_seconds);
        buffer.retain(|e| e.timestamp >= cutoff);
    }
});
//
   let app = Router::new()
   .route("/api/health", get(async || "OK"))
   .route("/api/system", get(system))
   .route("/api/network", get(network))
   .route("/api/network/{interface}", get(networkone))
   .route("/api/history", get(history))
   .with_state(AppState {
    history_buffer: history_buffer.clone(),
    primary_interface: config.primary_interface.clone(),
   });

   let listener = tokio::net::TcpListener::bind(&config.listen_address).await.unwrap();
   println!("Listening on {}", config.listen_address);
   axum::serve(listener, app).await.unwrap();
}

//to find cpu usage in two diffrent time intervals.
fn read_cpu() -> (u64, u64) {
    let content = std::fs::read_to_string("/proc/stat").unwrap();
    let line = content.lines().next().unwrap();
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|s| s.parse().unwrap())
        .collect();
    let idle = values[3];
    let total: u64 = values.iter().sum();
    (idle, total)
}

fn now_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn collect_snapshot() -> HistoryInfo {
    let uptime = std::fs::read_to_string("/proc/uptime").unwrap();
    let uptime_seconds: f64 = uptime.split_whitespace().nth(0).unwrap().parse().unwrap();

    let mem = std::fs::read_to_string("/proc/meminfo").unwrap();
    let mut total : u64= 0;
     let mut available : u64= 0;
    for line in mem.lines(){
        if line.starts_with("MemTotal:") {
         total = line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap();
        }
    }
        for line in mem.lines(){
        if line.starts_with("MemAvailable:") {
         available = line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap();
        }
    }
    
    let memory_used_kb = total - available;
    let memory_percent = ((memory_used_kb as f64 / total as f64 )* 100.0 * 100.0).round() / 100.0;

    let (idle1, total1) = read_cpu();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let (idle2, total2) = read_cpu();

    let idle_diff = idle2 - idle1;
    let total_diff = total2 - total1;

    let cpu_usage_percent = (((total_diff - idle_diff) as f64 / total_diff as f64) * 100.0 * 100.0).round() / 100.0;

    let cpu_temperature_celsius = match std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp") {
        Ok(temp) => Some(temp.trim().parse::<f64>().unwrap() / 1000.0),
        Err(_) => None,
    };

    HistoryInfo {
        timestamp: now_timestamp(),
        uptime_seconds,
        memory_total_kb: total,
        memory_used_kb,
        memory_percent,
        cpu_usage_percent,
        cpu_temperature_celsius,
    }
}

//system endpoint to return system information in json format.
async fn system() -> Json<SystemInfo> {

let snapshot = collect_snapshot().await;
    Json(SystemInfo {
        uptime_seconds: snapshot.uptime_seconds,
        memory_total_kb: snapshot.memory_total_kb,
        memory_used_kb: snapshot.memory_used_kb,
        memory_percent: snapshot.memory_percent,
        cpu_usage_percent: snapshot.cpu_usage_percent,
        cpu_temperature_celsius: snapshot.cpu_temperature_celsius,
    })
}

async fn network(State(state): State<AppState>) -> Json<Vec<InterfaceInfo>> {
    let content = std::fs::read_to_string("/proc/net/dev").unwrap();
    let mut interfaces = Vec::new();

    for line in content.lines().skip(2) {
        if let Some((name, data)) = line.split_once(':') {
            let name = name.trim().to_string();
            let values: Vec<u64> = data
                .split_whitespace()
                .map(|s| s.parse().unwrap())
                .collect();
            let rx_bytes = values[0];
            let tx_bytes = values[8];
            let link_state = std::fs::read_to_string(format!("/sys/class/net/{}/operstate", name))
                .unwrap_or_else(|_| "unknown".to_string())
                .trim()
                .to_string();
                let is_primary = name == state.primary_interface;

            interfaces.push(InterfaceInfo {
                name,
                rx_bytes,
                tx_bytes,
                link_state,
                is_primary: Some(is_primary),
            });
        }
    }

    Json(interfaces)
}

async fn networkone (Path(name): Path<String>) -> Result<Json<InterfaceInfo>, StatusCode> {
    let content = std::fs::read_to_string("/proc/net/dev").unwrap();

    for line in content.lines().skip(2) {
        if let Some((iface_name, data)) = line.split_once(':') {
            let iface_name = iface_name.trim().to_string();
            if iface_name != name {
                continue;
            }
                let values: Vec<u64> = data
                    .split_whitespace()
                    .map(|s| s.parse().unwrap())
                    .collect();
                let rx_bytes = values[0];
                let tx_bytes = values[8];
                let link_state = std::fs::read_to_string(format!("/sys/class/net/{}/operstate", name))
                    .unwrap_or_else(|_| "unknown".to_string())
                    .trim()
                    .to_string();

                return Ok(Json(InterfaceInfo {
                    name: iface_name,
                    rx_bytes,
                    tx_bytes,
                    link_state,
                    is_primary: None,
                }));
            
        }
    }
    Err(StatusCode::NOT_FOUND)
}

async fn history(State(state): State<AppState>,
Query(params): Query<HistoryQuery>,) -> Json<Vec<HistoryInfo>> {
    let minutes = params.minutes.unwrap_or(5);
    let cutoff = now_timestamp().saturating_sub(minutes * 60);
    let buffer = state.history_buffer.lock().unwrap();
    let results: Vec<HistoryInfo> = buffer.iter().filter(|e| e.timestamp >= cutoff).cloned().collect();
    Json(results)
}