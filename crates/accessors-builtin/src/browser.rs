//! `browser` 访问器。内核通过 `BrowserEngine` 调用，顶层导航先走出站端口。

use std::time::Instant;

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
    HttpRequest,
};
use async_trait::async_trait;
use base64::Engine;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use strata_model::{EngineInfo, ErrorClass, Locator, PartRole, RenderLevel};

use crate::envelope::{ExtraPart, Page, from_page};
use crate::http::url_of;
use crate::ready::{config, permanent, ready, spec_error};

pub struct EngineRequest {
    pub url: String,
    pub proxy: String,
    pub headers: Vec<(String, String)>,
    pub wait_until: Option<String>,
    pub eval: Option<String>,
    pub js: bool,
    pub frames: bool,
    pub layout: bool,
    pub resources: Vec<String>,
    pub trace: bool,
    pub matched_body: bool,
    pub block_private_networks: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MatchedBody {
    pub url: String,
    pub media_type: String,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct EngineOutput {
    pub final_url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub redirect_chain: Vec<String>,
    pub title: Option<String>,
    pub html: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
    pub network: Option<Vec<u8>>,
    pub matched: Vec<MatchedBody>,
    pub screenshot: Option<Vec<u8>>,
    pub pdf: Option<Vec<u8>>,
    pub renditions: Vec<(String, Vec<u8>)>,
    pub version: Option<String>,
}

#[async_trait]
pub trait BrowserEngine: Send + Sync {
    async fn fetch(&self, request: &EngineRequest) -> Result<EngineOutput, AccessError>;
}

pub struct BrowserAccessor<E> {
    engine: E,
    proxy: String,
}

impl<E> BrowserAccessor<E> {
    pub fn new(engine: E, proxy: impl Into<String>) -> Self {
        Self {
            engine,
            proxy: proxy.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Spec {
    #[serde(default = "moli_name")]
    engine: String,
    #[serde(default = "auto_mode")]
    mode: String,
    #[serde(default)]
    wait: Option<WaitSpec>,
    #[serde(default = "yes")]
    js: bool,
    #[serde(default)]
    frames: bool,
    #[serde(default)]
    layout: bool,
    #[serde(default)]
    resources: Vec<String>,
    #[serde(default)]
    eval: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    fallback: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitSpec {
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    script: Option<String>,
    #[serde(default)]
    response_url: Option<String>,
    #[serde(default)]
    response_url_regex: Option<String>,
    #[serde(default)]
    response_json: Option<String>,
    #[serde(default)]
    delay: Option<String>,
}

fn moli_name() -> String {
    "moli".into()
}

fn auto_mode() -> String {
    "auto".into()
}

fn yes() -> bool {
    true
}

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    let spec: Spec = if spec.is_null() {
        serde_json::from_value(json!({})).map_err(spec_error)?
    } else {
        serde_json::from_value(spec.clone()).map_err(spec_error)?
    };
    if spec.engine == "chromium" || spec.mode == "cdp" || spec.fallback.is_some() {
        return Err(config("未实测"));
    }
    if spec.engine != "moli" || (spec.mode != "auto" && spec.mode != "cli") {
        return Err(config("spec 无效"));
    }
    if let Some(wait) = &spec.wait {
        if wait.selector.is_some()
            || wait.script.is_some()
            || wait.response_url.is_some()
            || wait.response_url_regex.is_some()
            || wait.response_json.is_some()
            || wait.delay.is_some()
        {
            return Err(config("未实测"));
        }
        if let Some(until) = &wait.until {
            if !matches!(
                until.as_str(),
                "done" | "load" | "domcontentloaded" | "networkidle" | "domstable"
            ) {
                return Err(config("spec 无效"));
            }
        }
    }
    for kind in &spec.resources {
        if !matches!(kind.as_str(), "image" | "font" | "media") {
            return Err(config("未实测"));
        }
    }
    Ok(spec)
}

fn network_mode(policy: &Value) -> Result<&'static str, AccessError> {
    match policy
        .pointer("/capture/network")
        .and_then(Value::as_str)
        .unwrap_or("off")
    {
        "all-bodies" => Err(config("未实测")),
        "trace" => Ok("trace"),
        "matched-body" => Ok("matched-body"),
        _ => Ok("off"),
    }
}

fn wants_layout(policy: &Value, spec_layout: bool) -> bool {
    spec_layout
        || matches!(
            policy
                .pointer("/capture/screenshot")
                .and_then(Value::as_str),
            Some("on-failure" | "on-render" | "always")
        )
}

fn rendition_names(policy: &Value) -> Vec<String> {
    policy
        .pointer("/capture/renditions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn moli_args(request: &EngineRequest) -> Vec<String> {
    let mut args = vec![
        "fetch".into(),
        request.url.clone(),
        "--dump".into(),
        "json".into(),
        "--http-proxy".into(),
        request.proxy.clone(),
        "--block-private-networks".into(),
    ];
    if !request.js {
        args.push("--disable-js".into());
    }
    if request.frames {
        args.push("--with-frames".into());
    }
    if request.layout {
        args.push("--layout".into());
    }
    if let Some(eval) = &request.eval {
        args.push("--eval".into());
        args.push(eval.clone());
    }
    if let Some(until) = &request.wait_until {
        args.push("--wait-until".into());
        args.push(until.clone());
    }
    if request.trace {
        args.push("--trace-network".into());
    }
    if request.matched_body {
        args.push("--trace-matched-response-body".into());
    }
    for kind in &request.resources {
        args.push(format!("--{kind}"));
    }
    for (name, value) in &request.headers {
        args.push("-H".into());
        args.push(format!("{name}: {value}"));
    }
    args
}

pub fn parse_dump(bytes: &[u8]) -> Result<EngineOutput, AccessError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| permanent("engine"))?;
    let final_url = value
        .get("final_url")
        .and_then(Value::as_str)
        .ok_or_else(|| permanent("engine"))?
        .to_string();
    let status = u16::try_from(
        value
            .get("status")
            .and_then(Value::as_u64)
            .ok_or_else(|| permanent("engine"))?,
    )
    .map_err(|_| permanent("engine"))?;
    let headers = value
        .get("headers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let pair = item.as_array()?;
                    Some((
                        pair.first()?.as_str()?.to_string(),
                        pair.get(1)?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let redirect_chain = value
        .get("redirect_chain")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let html = value
        .get("html")
        .and_then(Value::as_str)
        .map(|text| text.as_bytes().to_vec());
    let body = value
        .get("body_base64")
        .and_then(Value::as_str)
        .map(|text| base64::engine::general_purpose::STANDARD.decode(text))
        .transpose()
        .map_err(|_| permanent("engine"))?;
    let network = value
        .get("network")
        .map(|item| serde_json::to_vec(item).unwrap_or_default());
    Ok(EngineOutput {
        final_url,
        status,
        headers,
        redirect_chain,
        title: value
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
        html,
        body,
        network,
        version: value
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..EngineOutput::default()
    })
}

pub struct MoliCommand {
    pub program: std::path::PathBuf,
}

#[async_trait]
impl BrowserEngine for MoliCommand {
    async fn fetch(&self, request: &EngineRequest) -> Result<EngineOutput, AccessError> {
        let output = tokio::process::Command::new(&self.program)
            .args(moli_args(request))
            .output()
            .await
            .map_err(|source| AccessError::with_source(ErrorClass::Permanent, "engine", source))?;
        if !output.status.success() {
            return Err(permanent("engine"));
        }
        parse_dump(&output.stdout)
    }
}

#[async_trait]
impl<E: BrowserEngine> Accessor for BrowserAccessor<E> {
    fn id(&self) -> &str {
        "browser"
    }

    fn version(&self) -> &str {
        "1"
    }

    fn capabilities(&self) -> &AccessorCapabilities {
        &CAPS
    }

    fn validate(&self, spec: &Value) -> Result<(), AccessError> {
        parse_spec(spec).map(|_| ())
    }

    async fn access(
        &self,
        request: AccessRequest,
        ctx: &AccessContext,
    ) -> Result<Vec<strata_model::RawEnvelope>, AccessError> {
        ready(ctx, request.deadline)?;
        if self.proxy.is_empty() {
            return Err(config("proxy"));
        }
        let spec = parse_spec(&request.spec)?;
        let mode = network_mode(&request.policy)?;
        let url = url_of(&request.locator)?;
        let mut headers = Vec::new();
        if let Some(profile) = &spec.session {
            let session = ctx.sessions.get(profile).await?;
            headers.extend(session.headers);
            if let Some(cookie) = session.cookie {
                if !headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("cookie"))
                {
                    headers.push(("Cookie".into(), cookie));
                }
            }
        }
        ctx.egress
            .request(HttpRequest {
                method: "GET".into(),
                url: url.clone(),
                headers: headers.clone(),
                body: Bytes::new(),
            })
            .await?;
        let layout = wants_layout(&request.policy, spec.layout);
        let trace = mode == "trace" || mode == "matched-body";
        let matched_body = mode == "matched-body";
        let output = self
            .engine
            .fetch(&EngineRequest {
                url,
                proxy: self.proxy.clone(),
                headers,
                wait_until: spec.wait.as_ref().and_then(|wait| wait.until.clone()),
                eval: spec.eval.clone(),
                js: spec.js,
                frames: spec.frames,
                layout,
                resources: spec.resources.clone(),
                trace,
                matched_body,
                block_private_networks: true,
            })
            .await?;
        let level = if layout || spec.eval.is_some() || trace {
            RenderLevel::L3
        } else {
            RenderLevel::L2
        };
        map_output(
            ctx,
            &request,
            output,
            level,
            &rendition_names(&request.policy),
            trace,
            matched_body,
        )
    }
}

fn map_output(
    ctx: &AccessContext,
    request: &AccessRequest,
    output: EngineOutput,
    level: RenderLevel,
    renditions: &[String],
    include_network: bool,
    include_matched: bool,
) -> Result<Vec<strata_model::RawEnvelope>, AccessError> {
    let document = if let Some(html) = output.html.clone() {
        Some((Bytes::from(html), "text/html".into()))
    } else {
        output.body.clone().map(|body| {
            let kind = crate::headers::media_type(&output.headers).0;
            (Bytes::from(body), kind)
        })
    };
    let metadata = json!({
        "finalUrl": output.final_url,
        "status": output.status,
        "headers": output.headers,
        "redirectChain": output.redirect_chain,
        "title": output.title,
    });
    let metadata_bytes = serde_json::to_vec(&metadata).map_err(|_| permanent("engine"))?;
    let mut extra = vec![ExtraPart {
        role: PartRole::Metadata,
        media_type: "application/json".into(),
        body: Bytes::from(metadata_bytes),
        locator: None,
        meta: Map::new(),
    }];
    if include_network {
        if let Some(network) = output.network.clone() {
            extra.push(ExtraPart {
                role: PartRole::NetworkTrace,
                media_type: "application/json".into(),
                body: Bytes::from(network),
                locator: None,
                meta: Map::new(),
            });
        }
    }
    if include_matched {
        for item in &output.matched {
            extra.push(ExtraPart {
                role: PartRole::NetworkResponse,
                media_type: item.media_type.clone(),
                body: Bytes::from(item.body.clone()),
                locator: Some(Locator::Url {
                    url: item.url.clone(),
                }),
                meta: Map::new(),
            });
        }
    }
    if let Some(screenshot) = output.screenshot.clone() {
        extra.push(ExtraPart {
            role: PartRole::Screenshot,
            media_type: "image/png".into(),
            body: Bytes::from(screenshot),
            locator: None,
            meta: Map::new(),
        });
    }
    if let Some(pdf) = output.pdf.clone() {
        extra.push(ExtraPart {
            role: PartRole::Print,
            media_type: "application/pdf".into(),
            body: Bytes::from(pdf),
            locator: None,
            meta: Map::new(),
        });
    }
    for (name, bytes) in &output.renditions {
        if renditions.iter().any(|wanted| wanted == name) {
            let mut meta = Map::new();
            meta.insert("derivedBy".into(), Value::String("moli".into()));
            extra.push(ExtraPart {
                role: PartRole::Rendition,
                media_type: "text/plain".into(),
                body: Bytes::from(bytes.clone()),
                locator: None,
                meta,
            });
        }
    }
    let (body, page_headers, has_document) = match document {
        Some((body, kind)) => {
            let mut page_headers = output.headers.clone();
            page_headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
            page_headers.push(("Content-Type".into(), kind));
            (body, page_headers, true)
        }
        None => (Bytes::new(), output.headers.clone(), false),
    };
    let started = Instant::now();
    let envelope = from_page(
        ctx,
        &request.policy,
        started,
        Page {
            locator: request.locator.clone(),
            final_url: output.final_url.clone(),
            status: output.status,
            headers: page_headers,
            body,
            cursor: None,
            receipt_id: None,
            render_level: Some(level),
            engine: Some(EngineInfo {
                name: "moli".into(),
                version: output
                    .version
                    .clone()
                    .unwrap_or_else(|| "unmeasured".into()),
            }),
            extra,
            document: has_document,
        },
    )?;
    Ok(vec![envelope])
}

fn caps() -> AccessorCapabilities {
    AccessorCapabilities {
        protocols: vec!["http".into(), "https".into()],
        conditional: false,
        render: true,
        layout: true,
        capture_network: CaptureNetwork::MatchedBody,
        interact: true,
        paid: None,
        batchable: false,
        inbound: false,
    }
}

static CAPS: std::sync::LazyLock<AccessorCapabilities> = std::sync::LazyLock::new(caps);
