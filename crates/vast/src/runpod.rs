//! RunPod GPU Pods backend. The router retains legacy numeric `vast_id` fields;
//! opaque RunPod IDs are deterministically mapped, never parsed as integers.
use crate::{CreateInstanceParams, Instance, Offer};
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub owner_id: String,
    pub gpu_type_ids: Vec<String>,
    pub cloud_type: String,
    pub min_ram_per_gpu: i64,
    pub min_vcpu_per_gpu: i64,
    pub container_disk_gb: i64,
    pub data_center_ids: Vec<String>,
    pub allowed_cuda_versions: Vec<String>,
    pub max_compute_usd_h: f64,
    /// Explicit running/stopped volume prices, USD per GB per month.
    /// Confirm current provider prices before deployment.
    pub volume_usd_gb_month: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            owner_id: String::new(),
            gpu_type_ids: vec![],
            cloud_type: "SECURE".into(),
            min_ram_per_gpu: 64,
            min_vcpu_per_gpu: 24,
            container_disk_gb: 20,
            data_center_ids: vec![],
            allowed_cuda_versions: vec!["13.0".into()],
            max_compute_usd_h: 0.90,
            volume_usd_gb_month: 0.20,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.owner_id.is_empty()
                && self
                    .owner_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "runpod.owner_id must be a stable alphanumeric identifier"
        );
        ensure!(
            !self.gpu_type_ids.is_empty(),
            "runpod.gpu_type_ids is required"
        );
        ensure!(
            matches!(self.cloud_type.as_str(), "SECURE" | "COMMUNITY"),
            "invalid runpod.cloud_type"
        );
        ensure!(
            self.min_ram_per_gpu > 0 && self.min_vcpu_per_gpu > 0 && self.container_disk_gb > 0,
            "invalid RunPod resource limits"
        );
        ensure!(
            self.max_compute_usd_h.is_finite() && self.max_compute_usd_h > 0.0,
            "RunPod compute ceiling must be positive"
        );
        ensure!(
            self.volume_usd_gb_month.is_finite() && self.volume_usd_gb_month > 0.0,
            "RunPod storage rate must be positive"
        );
        Ok(())
    }
}

#[derive(Clone)]
pub struct Runpod {
    cfg: Config,
    key: String,
    http: reqwest::Client,
    rest_root: String,
    graphql_root: String,
}

pub fn numeric_id(value: &str) -> i64 {
    let bytes = Sha256::digest(value.as_bytes());
    (i64::from_be_bytes(bytes[..8].try_into().unwrap()) & i64::MAX).max(1)
}

impl Runpod {
    pub fn new(key: String, cfg: Config) -> Result<Self> {
        Self::with_endpoints(
            key,
            cfg,
            "https://rest.runpod.io/v1",
            "https://api.runpod.io/graphql",
        )
    }
    pub fn with_endpoints(key: String, cfg: Config, rest: &str, graphql: &str) -> Result<Self> {
        cfg.validate()?;
        ensure!(!key.trim().is_empty(), "RUNPOD_API_KEY is required");
        for endpoint in [rest, graphql] {
            let url = reqwest::Url::parse(endpoint)?;
            ensure!(
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                "invalid RunPod endpoint"
            );
        }
        Ok(Self {
            cfg,
            key,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            rest_root: rest.trim_end_matches('/').into(),
            graphql_root: graphql.into(),
        })
    }
    async fn rest(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.rest_root))
            .bearer_auth(&self.key);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().await.context("RunPod REST transport")?;
        let status = response.status();
        // Do not echo response bodies: creation errors can include secret env values.
        ensure!(status.is_success(), "RunPod REST {path}: HTTP {status}");
        let text = response.text().await?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text).context("invalid RunPod REST JSON")?)
    }
    async fn pods(&self) -> Result<Vec<Value>> {
        let response = self
            .rest(reqwest::Method::GET, "/pods?includeMachine=true", None)
            .await?;
        let pods = response
            .as_array()
            .context("RunPod inventory must be an array; refusing partial inventory")?;
        let mut ids = std::collections::HashSet::new();
        let mut owned = vec![];
        for pod in pods {
            if pod.pointer("/env/PGPU_OWNER_ID").and_then(Value::as_str) != Some(&self.cfg.owner_id)
            {
                continue;
            }
            let id = pod["id"].as_str().context("owned RunPod pod missing ID")?;
            ensure!(
                ids.insert(numeric_id(id)),
                "RunPod numeric ID collision; refusing inventory"
            );
            owned.push(pod.clone());
        }
        Ok(owned)
    }
    async fn pod_id(&self, id: i64) -> Result<String> {
        self.pods()
            .await?
            .iter()
            .find(|p| p["id"].as_str().is_some_and(|s| numeric_id(s) == id))
            .and_then(|p| p["id"].as_str())
            .map(str::to_owned)
            .context("RunPod pod not found in this router's owned inventory")
    }
    pub async fn offers(&self, interruptible: bool, disk_gb: f64) -> Result<Vec<Offer>> {
        if interruptible {
            return Ok(vec![]);
        }
        let response = self.http.post(&self.graphql_root).bearer_auth(&self.key)
            .json(&json!({"query":"query { gpuTypes { id displayName memoryInGb securePrice communityPrice } }"}))
            .send().await.context("RunPod quote transport")?;
        ensure!(
            response.status().is_success(),
            "RunPod quote HTTP {}",
            response.status()
        );
        let value: Value = response.json().await?;
        ensure!(value.get("errors").is_none(), "RunPod quote query rejected");
        let types = value
            .pointer("/data/gpuTypes")
            .and_then(Value::as_array)
            .context("missing RunPod GPU quotes")?;
        let mut offers = vec![];
        for gpu in types {
            let id = gpu["id"].as_str().context("GPU quote missing ID")?;
            if !self.cfg.gpu_type_ids.iter().any(|s| s == id) {
                continue;
            }
            let field = if self.cfg.cloud_type == "SECURE" {
                "securePrice"
            } else {
                "communityPrice"
            };
            let Some(price) = gpu[field]
                .as_f64()
                .filter(|p| p.is_finite() && *p > 0.0 && *p <= self.cfg.max_compute_usd_h)
            else {
                continue;
            };
            let vram = gpu["memoryInGb"]
                .as_f64()
                .context("GPU quote missing VRAM")?;
            let storage = self.cfg.volume_usd_gb_month * disk_gb / 720.0;
            offers.push(Offer {
                id: numeric_id(id),
                gpu_name: id
                    .trim_start_matches("NVIDIA GeForce ")
                    .trim_start_matches("NVIDIA ")
                    .into(),
                gpu_ram: (vram * 1024.0) as i64,
                cpu_ram: self.cfg.min_ram_per_gpu * 1024,
                cpu_cores: Some(self.cfg.min_vcpu_per_gpu as f64),
                num_gpus: 1,
                disk_space: disk_gb,
                storage_cost: Some(self.cfg.volume_usd_gb_month),
                storage_total_cost: Some(storage),
                dph_base: Some(price),
                dph_total: Some(price + storage),
                machine_id: numeric_id(id),
                ..Default::default()
            });
        }
        Ok(offers)
    }
    pub fn create_body(&self, gpu_id: &str, params: &CreateInstanceParams<'_>) -> Result<Value> {
        ensure!(
            params.price.is_none(),
            "RunPod spot/bidding is not supported"
        );
        ensure!(
            self.cfg.gpu_type_ids.iter().any(|s| s == gpu_id),
            "GPU type not configured"
        );
        let mut env = params
            .env
            .as_object()
            .context("pod env must be an object")?
            .clone();
        ensure!(
            env.values().all(Value::is_string),
            "pod env values must be strings"
        );
        env.insert("PGPU_OWNER_ID".into(), json!(self.cfg.owner_id));
        env.insert("PGPU_GPU_TYPE_ID".into(), json!(gpu_id));
        // Keep models/configs on the persistent pod volume, not ephemeral root.
        env.insert("STRATA_DATA".into(), json!("/workspace/strata"));
        let mut body = json!({"name":params.label.context("pod label required")?, "imageName":params.image,
            "computeType":"GPU", "cloudType":self.cfg.cloud_type, "gpuTypeIds":[gpu_id], "gpuCount":1,
            "interruptible":false, "containerDiskInGb":self.cfg.container_disk_gb,
            "volumeInGb":params.disk.context("pod volume size required")?, "volumeMountPath":"/workspace",
            "minRAMPerGPU":self.cfg.min_ram_per_gpu, "minVCPUPerGPU":self.cfg.min_vcpu_per_gpu,
            "env":env, "ports":[], "dockerEntrypoint":[], "dockerStartCmd":[]});
        if !self.cfg.data_center_ids.is_empty() {
            body["dataCenterIds"] = json!(self.cfg.data_center_ids);
        }
        if !self.cfg.allowed_cuda_versions.is_empty() {
            body["allowedCudaVersions"] = json!(self.cfg.allowed_cuda_versions);
        }
        Ok(body)
    }
    pub async fn create(&self, offer_id: i64, params: &CreateInstanceParams<'_>) -> Result<i64> {
        let disk = params.disk.context("RunPod volume size required")?;
        // Requote immediately before renting. POST is never blindly retried.
        ensure!(
            self.offers(false, disk as f64)
                .await?
                .iter()
                .any(|o| o.id == offer_id),
            "RunPod quote missing or exceeds ceiling"
        );
        let gpu = self
            .cfg
            .gpu_type_ids
            .iter()
            .find(|s| numeric_id(s) == offer_id)
            .context("unknown RunPod GPU type")?;
        let pod = self
            .rest(
                reqwest::Method::POST,
                "/pods",
                Some(self.create_body(gpu, params)?),
            )
            .await?;
        let id = pod["id"].as_str().context(
            "RunPod create response missing pod ID; inspect provider console before retrying",
        )?;
        // REST has no atomic maximum-price option. Reject an unexpectedly high
        // returned compute price, deleting the newly created pod to stop charges.
        if !pod["costPerHr"]
            .as_f64()
            .is_some_and(|p| p.is_finite() && p > 0.0 && p <= self.cfg.max_compute_usd_h)
        {
            self.rest(reqwest::Method::DELETE, &format!("/pods/{id}"), None)
                .await
                .with_context(|| {
                    format!(
                        "Unverified/overpriced pod {id}; automatic cleanup FAILED, delete manually"
                    )
                })?;
            bail!("RunPod returned missing/over-ceiling compute price; created pod deleted");
        }
        Ok(numeric_id(id))
    }
    pub async fn instances(&self) -> Result<Vec<Instance>> {
        let mut output = vec![];
        for p in self.pods().await? {
            let id = p["id"].as_str().context("pod missing ID")?;
            let status = match p["desiredStatus"].as_str() {
                Some("RUNNING") => "running", // conservative billable allocation; NOT model readiness
                Some("EXITED") => "stopped",
                Some("TERMINATED") => continue,
                _ => bail!("unknown RunPod desired status; refusing inventory"),
            };
            // Agent heartbeat establishes actual readiness separately.
            output.push(Instance {
                id: numeric_id(id),
                label: p["name"].as_str().map(str::to_owned),
                gpu_name: p
                    .pointer("/machine/gpuTypeId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                image: p["image"].as_str().map(str::to_owned),
                actual_status: Some(status.into()),
                intended_status: Some(
                    if status == "stopped" {
                        "stopped"
                    } else {
                        "running"
                    }
                    .into(),
                ),
                dph_base: p["costPerHr"].as_f64(),
                dph_total: p["costPerHr"].as_f64(),
                storage_cost: Some(self.cfg.volume_usd_gb_month),
                storage_total: p["volumeInGb"].as_f64(),
                extra_env: p["env"].clone(),
                ..Default::default()
            });
        }
        Ok(output)
    }
    pub async fn set_status(&self, id: i64, running: bool) -> Result<()> {
        let pod = self.pod_id(id).await?;
        if running {
            let current = self
                .rest(reqwest::Method::GET, &format!("/pods/{pod}"), None)
                .await?;
            let gpu = current
                .pointer("/env/PGPU_GPU_TYPE_ID")
                .and_then(Value::as_str)
                .context("RunPod pod missing original GPU type")?;
            let disk = current["volumeInGb"]
                .as_f64()
                .context("RunPod pod missing volume size")?;
            ensure!(
                self.offers(false, disk)
                    .await?
                    .iter()
                    .any(|o| o.id == numeric_id(gpu)),
                "RunPod resume quote missing/over ceiling"
            );
        }
        self.rest(
            reqwest::Method::POST,
            &format!("/pods/{pod}/{}", if running { "start" } else { "stop" }),
            None,
        )
        .await?;
        if running {
            let started = self
                .rest(reqwest::Method::GET, &format!("/pods/{pod}"), None)
                .await?;
            if !started["costPerHr"]
                .as_f64()
                .is_some_and(|p| p.is_finite() && p > 0.0 && p <= self.cfg.max_compute_usd_h)
            {
                self.rest(reqwest::Method::POST, &format!("/pods/{pod}/stop"), None).await.with_context(|| format!("Overpriced/unverified resumed pod {pod}: cleanup failed; stop manually"))?;
                bail!("RunPod resume price unverified/over ceiling; pod stopped");
            }
        }
        Ok(())
    }
    pub async fn destroy(&self, id: i64) -> Result<()> {
        let pods = self.pods().await?;
        let Some(pod) = pods
            .iter()
            .find(|p| p["id"].as_str().is_some_and(|s| numeric_id(s) == id))
        else {
            return Ok(());
        };
        let raw = pod["id"].as_str().context("pod missing ID")?;
        self.rest(reqwest::Method::DELETE, &format!("/pods/{raw}"), None)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "runpod_tests.rs"]
mod contract_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_positive_ids() {
        assert_eq!(numeric_id("pod-abc"), numeric_id("pod-abc"));
        assert!(numeric_id("pod-abc") > 0);
        assert_ne!(numeric_id("pod-abc"), numeric_id("pod-def"));
    }
    #[test]
    fn validate_settings() {
        assert!(Config::default().validate().is_err());
        let mut c = Config::default();
        c.owner_id = "router-one".into();
        c.gpu_type_ids = vec!["NVIDIA GeForce RTX 5090".into()];
        assert!(c.validate().is_ok());
        c.max_compute_usd_h = f64::NAN;
        assert!(c.validate().is_err());
    }
}
