// 常见模型平台的快速配置预设：只填充端点、密钥环境变量名、模型名候选与能力缺省，
// 密钥值仍走环境变量注入或用户直接填写（直接填写的值只写不读）。
// 候选按各平台 2026-10 文档的全量在售型号列出（同代不同档位都保留），
// 平台上新时优先用「拉取平台模型」按端点现场发现，避免候选清单滞后。
// - OpenAI：GPT-6 系列（astra / 6.1-sol / 6-sol / 6-luna）1.05M 上下文、128K 输出；
//   GPT-5.6 三档（sol / terra / luna）与 GPT-5.5 仍可调用；
// - Anthropic：Claude 5.5 系列（Opus / Sonnet / Haiku）1M 上下文、128K 输出，
//   4.6 / 4.5 代仍在售；
// - Google：Gemini 4 Argon 1M 上下文与 1M 长解码，Gemini 3.8 Flash / Live，2.5 代仍在售；
// - xAI：Grok 4.7 / 4.6 / 4.5 / 4.3；
// - DeepSeek：deepseek-flash（V4.1 Flash，多模态）、deepseek-v4-pro、deepseek-v4-flash-vision-exp；
// - Moonshot：kimi-k3.1-pro、kimi-k3.1-pro-highspeed、kimi-k3、kimi-k2.7-code、
//   kimi-k2.7-code-highspeed、kimi-k2.6；
// - 百炼：qwen3.8-max / qwen3.8-flash / qwen3.7-plus / qwen3.8-omni-flash；
// - 智谱：GLM-5.3 / 5.3-Flash / 5.3-FlashX / 5.2 / 5.1 / 4.7；
// - 火山方舟：doubao-seed-2.1-pro（含 260915 长版）、doubao-seed-2.1-turbo、doubao-seed-evolving；
// - MiniMax：M3.1-Flash（含 Preview）、M3、M2.7、M2.5。
// 已下线模型（deepseek-chat/reasoner、gpt-4o/4.1、kimi-k2 与 moonshot-v1 系列）不再作为候选。
export interface ModelProfile {
  /** 上下文窗口（tokens），登记后作为该模型的缺省预算。 */
  contextWindow: number;
  /** 单次输出上限（tokens），登记后作为该模型的缺省值。 */
  maxTokens: number;
  /** 是否支持思考模式（推理）输出。 */
  reasoning: boolean;
  /** 支持的输入模态，至少包含 text。 */
  modalities: string[];
  /** 采样温度；null 表示按厂商官方缺省、不向请求注入 temperature。 */
  temperature: number | null;
}

export interface ModelPreset {
  id: string;
  label: string;
  kind: "remoteApi" | "localService" | "localCli";
  endpoint: string;
  secretRef: string;
  models: string[];
}

export const MODEL_PRESETS: ModelPreset[] = [
  {
    id: "openai",
    label: "OpenAI",
    kind: "remoteApi",
    endpoint: "https://api.openai.com/v1",
    secretRef: "OPENAI_API_KEY",
    models: ["gpt-6-astra", "gpt-6.1-sol", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5"],
  },
  {
    id: "anthropic",
    label: "Anthropic（Claude，OpenAI 兼容层）",
    kind: "remoteApi",
    endpoint: "https://api.anthropic.com/v1",
    secretRef: "ANTHROPIC_API_KEY",
    models: [
      "claude-opus-5-5",
      "claude-sonnet-5-5",
      "claude-haiku-5-5",
      "claude-opus-4-6",
      "claude-sonnet-4-6",
      "claude-opus-4-5",
      "claude-sonnet-4-5",
      "claude-haiku-4-5",
    ],
  },
  {
    id: "google",
    label: "Google Gemini（OpenAI 兼容层）",
    kind: "remoteApi",
    endpoint: "https://generativelanguage.googleapis.com/v1beta/openai/",
    secretRef: "GEMINI_API_KEY",
    models: ["gemini-4-argon", "gemini-3.8-flash", "gemini-3.8-live", "gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.5-flash-lite"],
  },
  {
    id: "xai",
    label: "xAI（Grok）",
    kind: "remoteApi",
    endpoint: "https://api.x.ai/v1",
    secretRef: "XAI_API_KEY",
    models: ["grok-4.7", "grok-4.6", "grok-4.5", "grok-4.3"],
  },
  {
    id: "deepseek",
    label: "DeepSeek",
    kind: "remoteApi",
    endpoint: "https://api.deepseek.com",
    secretRef: "DEEPSEEK_API_KEY",
    models: ["deepseek-flash", "deepseek-v4-pro", "deepseek-v4-flash", "deepseek-v4-flash-vision-exp"],
  },
  {
    id: "moonshot",
    label: "Moonshot（Kimi）",
    kind: "remoteApi",
    endpoint: "https://api.moonshot.cn/v1",
    secretRef: "MOONSHOT_API_KEY",
    models: [
      "kimi-k3.1-pro",
      "kimi-k3.1-pro-highspeed",
      "kimi-k3",
      "kimi-k2.7-code",
      "kimi-k2.7-code-highspeed",
      "kimi-k2.6",
    ],
  },
  {
    id: "dashscope",
    label: "通义千问（百炼 DashScope 兼容模式）",
    kind: "remoteApi",
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    secretRef: "DASHSCOPE_API_KEY",
    models: ["qwen3.8-max", "qwen3.8-flash", "qwen3.7-plus", "qwen3.8-omni-flash"],
  },
  {
    id: "zhipu",
    label: "智谱 GLM",
    kind: "remoteApi",
    endpoint: "https://open.bigmodel.cn/api/paas/v4",
    secretRef: "ZHIPUAI_API_KEY",
    models: ["glm-5.3", "glm-5.3-flash", "glm-5.3-flashx", "glm-5.2", "glm-5.1", "glm-4.7"],
  },
  {
    id: "volcano-ark",
    label: "豆包（火山方舟 Ark）",
    kind: "remoteApi",
    endpoint: "https://ark.cn-beijing.volces.com/api/v3",
    secretRef: "ARK_API_KEY",
    models: [
      "doubao-seed-2-1-pro-260915",
      "doubao-seed-2-1-pro",
      "doubao-seed-2-1-turbo",
      "doubao-seed-evolving",
    ],
  },
  {
    id: "minimax",
    label: "MiniMax",
    kind: "remoteApi",
    endpoint: "https://api.minimax.io/v1",
    secretRef: "MINIMAX_API_KEY",
    models: ["MiniMax-M3.1-Flash", "MiniMax-M3.1-Flash-Preview", "MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.5"],
  },
  {
    id: "ollama",
    label: "Ollama（本机）",
    kind: "localService",
    endpoint: "http://127.0.0.1:11434/v1",
    secretRef: "",
    models: ["qwen3", "llama3.2", "gemma3"],
  },
  {
    id: "lm-studio",
    label: "LM Studio（本机）",
    kind: "localService",
    endpoint: "http://127.0.0.1:1234/v1",
    secretRef: "",
    models: ["local-model"],
  },
];

export const CUSTOM_PRESET: ModelPreset = {
  id: "custom",
  label: "自定义（OpenAI 兼容端点）",
  kind: "remoteApi",
  endpoint: "",
  secretRef: "",
  models: [],
};

/** OpenAI GPT-6 系列共用的能力面：1.05M 上下文、128K 输出、文本与图像输入。
 * 思考型模型不接受自定义 temperature，采样一律交给官方缺省。 */
const GPT6: ModelProfile = { contextWindow: 1_050_000, maxTokens: 128_000, reasoning: true, modalities: ["text", "image"], temperature: null };
/** Claude 5.5 系列：1M 上下文、128K 输出、文本与图像输入；开启思考时温度必须为官方值。 */
const CLAUDE_55: ModelProfile = { contextWindow: 1_000_000, maxTokens: 128_000, reasoning: true, modalities: ["text", "image"], temperature: null };
/** Kimi K3 系列：1M 上下文，输出缺省取官方 max_completion_tokens 默认值 131072。 */
const KIMI_K3: ModelProfile = { contextWindow: 1_000_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null };
/** Qwen 3.8 旗舰：1M 上下文、131072 输出、思考模式。 */
const QWEN_MAX: ModelProfile = { contextWindow: 1_000_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null };
/** GLM-5.3：1M 上下文、128K 输出。 */
const GLM_53: ModelProfile = { contextWindow: 1_000_000, maxTokens: 131_072, reasoning: true, modalities: ["text"], temperature: null };
/** 豆包 Seed 2.1：长版 1.02M，早期 256K 版本共享输入输出窗口。 */
const DOUBAO_PRO_LONG: ModelProfile = { contextWindow: 1_024_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null };
const DOUBAO_PRO: ModelProfile = { contextWindow: 256_000, maxTokens: 65_536, reasoning: true, modalities: ["text", "image"], temperature: null };
/** MiniMax M3 系列：1M 上下文、可调思考深度。 */
const MINIMAX_M3: ModelProfile = { contextWindow: 1_000_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null };
/** GPT-5.6 三档与 GPT-5.5：400K 上下文、128K 输出，思考型不注入温度。 */
const GPT5X: ModelProfile = { contextWindow: 400_000, maxTokens: 128_000, reasoning: true, modalities: ["text", "image"], temperature: null };

/** 能力缺省库：按模型名给出与官方规格匹配的上下文、输出上限、思考模式与输入模态。 */
const PROFILES: Record<string, ModelProfile> = {
  "gpt-6-astra": GPT6,
  "gpt-6.1-sol": GPT6,
  "gpt-6-sol": GPT6,
  "gpt-6-luna": GPT6,
  "gpt-5.6-sol": GPT5X,
  "gpt-5.6-terra": GPT5X,
  "gpt-5.6-luna": GPT5X,
  "gpt-5.5": GPT5X,
  "claude-opus-5-5": CLAUDE_55,
  "claude-sonnet-5-5": CLAUDE_55,
  "claude-haiku-5-5": CLAUDE_55,
  "claude-opus-4-6": { contextWindow: 1_000_000, maxTokens: 32_768, reasoning: true, modalities: ["text", "image"], temperature: null },
  "claude-sonnet-4-6": { contextWindow: 1_000_000, maxTokens: 16_384, reasoning: true, modalities: ["text", "image"], temperature: null },
  "claude-opus-4-5": { contextWindow: 200_000, maxTokens: 64_000, reasoning: true, modalities: ["text", "image"], temperature: null },
  "claude-sonnet-4-5": { contextWindow: 200_000, maxTokens: 64_000, reasoning: true, modalities: ["text", "image"], temperature: null },
  "claude-haiku-4-5": { contextWindow: 200_000, maxTokens: 32_000, reasoning: true, modalities: ["text", "image"], temperature: null },
  "gemini-4-argon": { contextWindow: 1_000_000, maxTokens: 1_000_000, reasoning: true, modalities: ["text", "image", "audio", "video"], temperature: 1 },
  "gemini-3.8-flash": { contextWindow: 1_048_576, maxTokens: 65_536, reasoning: true, modalities: ["text", "image", "audio", "video"], temperature: 1 },
  "gemini-3.8-live": { contextWindow: 1_048_576, maxTokens: 65_536, reasoning: true, modalities: ["text", "image", "audio", "video"], temperature: 1 },
  "gemini-2.5-pro": { contextWindow: 1_048_576, maxTokens: 65_536, reasoning: true, modalities: ["text", "image", "audio", "video"], temperature: 1 },
  "gemini-2.5-flash": { contextWindow: 1_048_576, maxTokens: 65_536, reasoning: true, modalities: ["text", "image", "audio", "video"], temperature: 1 },
  "gemini-2.5-flash-lite": { contextWindow: 1_048_576, maxTokens: 65_536, reasoning: false, modalities: ["text", "image", "audio", "video"], temperature: 1 },
  // Grok 4.7 官方标注无文本输出硬上限；缺省取保守值，避免超长 max_tokens 被拒。采样走官方缺省。
  "grok-4.7": { contextWindow: 500_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null },
  "grok-4.6": { contextWindow: 500_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null },
  "grok-4.5": { contextWindow: 500_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null },
  "grok-4.3": { contextWindow: 1_000_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null },
  "deepseek-flash": { contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true, modalities: ["text", "image"], temperature: null },
  "deepseek-v4-pro": { contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true, modalities: ["text"], temperature: null },
  "deepseek-v4-flash": { contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true, modalities: ["text", "image"], temperature: null },
  "deepseek-v4-flash-vision-exp": { contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true, modalities: ["text", "image"], temperature: null },
  "kimi-k3.1-pro": KIMI_K3,
  "kimi-k3.1-pro-highspeed": KIMI_K3,
  "kimi-k3": KIMI_K3,
  "kimi-k2.7-code": { contextWindow: 256_000, maxTokens: 32_768, reasoning: true, modalities: ["text"], temperature: null },
  "kimi-k2.7-code-highspeed": { contextWindow: 256_000, maxTokens: 32_768, reasoning: true, modalities: ["text"], temperature: null },
  "kimi-k2.6": { contextWindow: 256_000, maxTokens: 32_768, reasoning: true, modalities: ["text", "image"], temperature: null },
  "qwen3.8-max": QWEN_MAX,
  "qwen3.8-flash": { contextWindow: 1_000_000, maxTokens: 65_536, reasoning: true, modalities: ["text", "image"], temperature: null },
  "qwen3.7-plus": { contextWindow: 1_000_000, maxTokens: 65_536, reasoning: true, modalities: ["text", "image"], temperature: null },
  "qwen3.8-omni-flash": { contextWindow: 1_000_000, maxTokens: 65_536, reasoning: true, modalities: ["text", "image", "audio", "video"], temperature: null },
  "glm-5.3": GLM_53,
  "glm-5.3-flash": { ...GLM_53, modalities: ["text", "image"] },
  "glm-5.3-flashx": { ...GLM_53, modalities: ["text", "image", "video"] },
  "glm-5.2": GLM_53,
  "glm-5.1": GLM_53,
  "glm-4.7": { contextWindow: 200_000, maxTokens: 131_072, reasoning: true, modalities: ["text"], temperature: null },
  "doubao-seed-2-1-pro-260915": DOUBAO_PRO_LONG,
  "doubao-seed-2-1-pro": DOUBAO_PRO,
  "doubao-seed-2-1-turbo": DOUBAO_PRO,
  "doubao-seed-evolving": { contextWindow: 1_024_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null },
  "minimax-m3.1-flash": MINIMAX_M3,
  "minimax-m3.1-flash-preview": MINIMAX_M3,
  "minimax-m3": MINIMAX_M3,
  "minimax-m2.7": { contextWindow: 204_800, maxTokens: 131_072, reasoning: true, modalities: ["text"], temperature: null },
  "minimax-m2.5": { contextWindow: 204_800, maxTokens: 131_072, reasoning: true, modalities: ["text"], temperature: null },
  qwen3: { contextWindow: 32_768, maxTokens: 8_192, reasoning: true, modalities: ["text"], temperature: null },
  "llama3.2": { contextWindow: 131_072, maxTokens: 4_096, reasoning: false, modalities: ["text"], temperature: 0.7 },
  gemma3: { contextWindow: 131_072, maxTokens: 8_192, reasoning: false, modalities: ["text", "image"], temperature: 0.7 },
  "local-model": { contextWindow: 32_768, maxTokens: 2_048, reasoning: false, modalities: ["text"], temperature: 0.7 },
};

/** 未知模型按通用保守值兜底：不猜能力，避免给出比实际更大的预算。 */
export const FALLBACK_PROFILE: ModelProfile = {
  contextWindow: 32_768,
  maxTokens: 2_048,
  reasoning: false,
  modalities: ["text"],
  temperature: 0.7,
};

/** 按模型名前缀识别家族，用于新版本型号不在预设库时自动带出能力。 */
function familyProfile(model: string): ModelProfile | null {
  const name = model.toLowerCase();
  if (name.startsWith("gpt-6")) return GPT6;
  if (name.startsWith("gpt-5")) return GPT5X;
  if (name.startsWith("gpt-4") || name.startsWith("o3") || name.startsWith("o4")) {
    return { contextWindow: 200_000, maxTokens: 100_000, reasoning: true, modalities: ["text", "image"], temperature: null };
  }
  if (name.startsWith("claude")) return CLAUDE_55;
  if (name.startsWith("gemini")) return { contextWindow: 1_048_576, maxTokens: 65_536, reasoning: true, modalities: ["text", "image", "audio", "video"], temperature: 1 };
  if (name.startsWith("grok")) return { contextWindow: 500_000, maxTokens: 131_072, reasoning: true, modalities: ["text", "image"], temperature: null };
  if (name.startsWith("deepseek")) return { contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true, modalities: ["text"], temperature: null };
  if (name.startsWith("kimi") || name.startsWith("moonshot")) return KIMI_K3;
  if (name.startsWith("qwen")) return QWEN_MAX;
  if (name.startsWith("glm")) return GLM_53;
  if (name.startsWith("doubao")) return DOUBAO_PRO;
  if (name.startsWith("minimax")) return MINIMAX_M3;
  if (name.startsWith("llama") || name.startsWith("mistral") || name.startsWith("gemma")) {
    return { contextWindow: 128_000, maxTokens: 8_192, reasoning: false, modalities: ["text"], temperature: 0.7 };
  }
  return null;
}

/** 能力缺省解析：精确命中预设 → 家族前缀 → 通用兜底。 */
export function resolveProfile(model: string): ModelProfile {
  const key = model.trim().toLowerCase();
  if (!key) return FALLBACK_PROFILE;
  const exact = PROFILES[key];
  if (exact) return exact;
  return familyProfile(key) ?? FALLBACK_PROFILE;
}

/** 能力摘要：用于列表与表单提示，避免同一段拼接在多处重复。 */
export function profileSummary(profile: ModelProfile): string {
  const window = profile.contextWindow >= 1_000_000
    ? `${(profile.contextWindow / 1_000_000).toFixed(2).replace(/\.?0+$/, "")}M`
    : `${Math.round(profile.contextWindow / 1_000)}k`;
  return [
    `上下文 ${window}`,
    `输出 ≤${profile.maxTokens}`,
    profile.reasoning ? "支持思考" : "不开思考",
    profile.modalities.join("/"),
    profile.temperature === null ? "采样 官方缺省" : `温度 ${profile.temperature}`,
  ].join(" · ");
}
