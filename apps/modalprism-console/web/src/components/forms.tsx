// 表单原语。
//
// 声明编辑、模型编辑与工作流编辑共用同一套控件：标签、说明、输入框的
// 间距与字号只在这里定义一次，页面里不再各写一套行内样式。

import type { ReactNode } from "react";

export function Field({
  label,
  hint,
  wide = false,
  children,
}: {
  label: ReactNode;
  hint?: ReactNode;
  wide?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={`form-field${wide ? " is-wide" : ""}`}>
      <span className="form-label">{label}</span>
      {children}
      {hint ? <span className="form-hint">{hint}</span> : null}
    </div>
  );
}

export function TextField({
  value,
  onChange,
  placeholder,
  mono = false,
  disabled = false,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  mono?: boolean;
  disabled?: boolean;
}) {
  return (
    <input
      className={`form-input${mono ? " mono" : ""}`}
      value={value}
      placeholder={placeholder}
      disabled={disabled}
      spellCheck={false}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

export function NumberField({
  value,
  onChange,
  placeholder,
  min,
}: {
  value: number | null;
  onChange: (value: number | null) => void;
  placeholder?: string;
  min?: number;
}) {
  return (
    <input
      className="form-input"
      type="number"
      min={min}
      value={value === null ? "" : value}
      placeholder={placeholder}
      onChange={(event) => {
        const text = event.target.value.trim();
        const parsed = Number(text);
        onChange(text === "" || Number.isNaN(parsed) ? null : parsed);
      }}
    />
  );
}

export function TextArea({
  value,
  onChange,
  placeholder,
  rows = 5,
  mono = false,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  rows?: number;
  mono?: boolean;
}) {
  return (
    <textarea
      className={`form-textarea${mono ? " mono" : ""}`}
      rows={rows}
      value={value}
      placeholder={placeholder}
      spellCheck={false}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

export function SelectField({
  value,
  onChange,
  options,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
  placeholder?: string;
}) {
  return (
    <select
      className="form-select"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    >
      {placeholder !== undefined ? <option value="">{placeholder}</option> : null}
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

/** 开关：声明启用状态、通知时机都用它。 */
export function Switch({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={`switch${checked ? " is-on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="switch-track" aria-hidden="true">
        <span className="switch-knob" />
      </span>
      <span className="switch-text">{checked ? "启用" : "停用"}</span>
    </button>
  );
}

/** 分段按钮：模式选择这类 2~4 选一。 */
export function Choice<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div className="seg" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={option.value === value ? "is-active" : undefined}
          aria-pressed={option.value === value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
