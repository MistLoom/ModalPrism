// 外部平台的轻量图标：用首字母/缩写与固定色相区分，避免引入外部图片资源。
// 图标键由后端目录给出（search engines、通知平台），前端只做映射与兜底。

const COLORS: Record<string, string> = {
  openai: "#10a37f",
  deepseek: "#4d6bfe",
  moonshot: "#1f2937",
  dashscope: "#6f42c1",
  zhipu: "#2563eb",
  "volcano-ark": "#0ea5e9",
  ollama: "#111827",
  "lm-studio": "#4b5563",
  ddg: "#de5833",
  bing: "#00809d",
  google: "#1a73e8",
  serpapi: "#16a34a",
  feishu: "#3370ff",
  wecom: "#2f7feb",
  dingtalk: "#1f9cf0",
  slack: "#4a154b",
  discord: "#5865f2",
  telegram: "#229ed9",
  webhook: "#6b7280",
  model: "#0f766e",
  endpoint: "#7c3aed",
  channel: "#b45309",
};

const GLYPHS: Record<string, string> = {
  ddg: "D",
  bing: "b",
  google: "G",
  serpapi: "S",
  feishu: "飞",
  wecom: "企",
  dingtalk: "钉",
  slack: "S",
  discord: "D",
  telegram: "T",
  webhook: "W",
  model: "M",
  endpoint: "API",
  channel: "渠",
};

export function PlatformIcon({ icon, label, size = 34 }: { icon: string; label?: string; size?: number }) {
  const key = icon || "webhook";
  const glyph = GLYPHS[key] ?? (label ? label.slice(0, 1).toUpperCase() : "•");
  const color = COLORS[key] ?? "#64748b";
  const fontSize = glyph.length > 1 ? size * 0.3 : size * 0.42;
  return (
    <span
      className="platform-icon"
      aria-hidden="true"
      style={{
        width: size,
        height: size,
        background: `${color}14`,
        color,
        borderColor: `${color}33`,
        fontSize,
      }}
    >
      {glyph}
    </span>
  );
}

export function PlatformBadge({ icon, name }: { icon: string; name: string }) {
  return (
    <span className="platform-badge">
      <PlatformIcon icon={icon} label={name} size={22} />
      <span>{name}</span>
    </span>
  );
}
