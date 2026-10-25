// 声明编辑的共用部件。
//
// 信源页与采集通道页面对的是同一件事：三类 YAML 声明的增删改。模板新建、批量操作、
// 代理取值与字段表单都在这里实现一次；页面只决定显示哪一类、选中了谁。

import { useEffect, useState } from "react";

import { Drawer } from "./Drawer";
import { Field, NumberField, SelectField, TextArea, TextField } from "./forms";
import { IconAlert, IconTrash } from "./icons";
import { useConsole } from "../lib/console";
import {
  createDeclaration,
  fieldsOf,
  proxyMode,
  proxyOf,
  readPath,
  removeDeclaration,
  templates,
  updateDeclaration,
  withProxy,
  writePath,
  type Declaration,
  type DeclarationKind,
  type Template,
} from "../lib/catalog";

/** 新建草稿：模板给出的正文与建议的文件名。 */
export interface DeclarationSeed {
  slug: string;
  doc: Record<string, unknown>;
}

export function ProxyInput({
  value,
  onChange,
  note,
}: {
  value: string | null;
  onChange: (value: string | null) => void;
  note?: string;
}) {
  const mode = proxyMode(value);
  return (
    <div className="proxy-input">
      <select
        className="form-select"
        value={mode}
        aria-label="代理方式"
        onChange={(event) => {
          const next = event.target.value;
          if (next === "inherit") {
            onChange(null);
          } else if (next === "off") {
            onChange("off");
          } else {
            onChange("http://127.0.0.1:7890");
          }
        }}
      >
        <option value="inherit">跟随系统</option>
        <option value="off">直连</option>
        <option value="manual">指定代理</option>
      </select>
      {mode === "manual" ? (
        <input
          className="form-input mono"
          value={value ?? ""}
          placeholder="http://127.0.0.1:7890"
          spellCheck={false}
          aria-label="代理地址"
          onChange={(event) => onChange(event.target.value)}
        />
      ) : (
        <span className="form-hint">{note ?? (mode === "off" ? "绕过系统代理" : "沿用系统设置")}</span>
      )}
    </div>
  );
}

/** 模板选择：新建声明时不写 YAML，先挑一个形态。 */
export function TemplatePicker({
  kind,
  onPick,
  onClose,
}: {
  kind: DeclarationKind;
  onPick: (template: DeclarationSeed) => void;
  onClose: () => void;
}) {
  const { notify } = useConsole();
  const [rows, setRows] = useState<Template[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    templates()
      .then((list) => {
        if (alive) {
          setRows(list.filter((item) => item.kind === kind));
        }
      })
      .catch((problem: Error) => {
        if (alive) {
          setFailure(problem.message);
        }
      });
    return () => {
      alive = false;
    };
  }, [kind]);

  return (
    <Drawer open kicker="新建声明" title="挑一个起点" onClose={onClose}>
      {failure ? (
        <div className="note is-bad">
          <IconAlert size={14} />
          <span>读取模板失败：{failure}</span>
        </div>
      ) : rows === null ? (
        <p className="muted">正在读取模板…</p>
      ) : rows.length === 0 ? (
        <p className="muted">这一类没有模板，可以直接手写一份空白声明。</p>
      ) : (
        <div className="template-grid">
          {rows.map((template) => (
            <button
              key={template.slug}
              type="button"
              className="template-card"
              onClick={() => {
                notify("info", `已套用模板「${template.name}」`, "改好地址与名称后保存，校验不通过会自动回滚。");
                onPick({ slug: template.slug, doc: template.doc });
              }}
            >
              <span className="template-name">{template.name}</span>
              <span className="template-desc">{template.description}</span>
              <span className="template-tag mono">{template.kind}/{template.slug}</span>
            </button>
          ))}
        </div>
      )}
    </Drawer>
  );
}

/** 声明抽屉：新建与编辑共用一张表单，底部留一个整份 JSON 的出口。 */
export function DeclarationDrawer({
  kind,
  item,
  seed,
  accessors,
  onClose,
  onSaved,
}: {
  kind: DeclarationKind;
  item: Declaration | null;
  seed: DeclarationSeed | null;
  accessors: Declaration[];
  onClose: () => void;
  onSaved: (message: string) => void;
}) {
  const { notify } = useConsole();
  const editing = item !== null;
  const [slug, setSlug] = useState(item?.slug ?? seed?.slug ?? "");
  const [doc, setDoc] = useState<Record<string, unknown>>(
    item?.doc ?? seed?.doc ?? { name: "新的声明" },
  );
  const [advanced, setAdvanced] = useState(false);
  const [draft, setDraft] = useState(() => JSON.stringify(item?.doc ?? seed?.doc ?? {}, null, 2));
  const [saving, setSaving] = useState(false);
  const fields = fieldsOf(kind, accessors);
  const supportsProxy = kind !== "adapter";
  const removable = !(kind === "adapter" && item?.removable === false);

  function update(path: string[], value: unknown) {
    setDoc((current) => writePath(current, path, value));
  }

  async function submit() {
    if (!slug.trim()) {
      notify("bad", "还缺声明名", "声明名会成为文件名，只能用字母、数字、点、下划线与连字符。");
      return;
    }
    setSaving(true);
    try {
      const draftDoc = { kind, slug: slug.trim(), doc };
      const saved = editing ? await updateDeclaration(draftDoc) : await createDeclaration(draftDoc);
      onSaved(editing ? `已更新 ${saved.file}` : `已新建 ${saved.file}`);
      onClose();
    } catch (problem) {
      notify("bad", editing ? "改写没有通过校验" : "新建没有通过校验", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  async function discard() {
    if (!item) {
      return;
    }
    if (!window.confirm(`删除 ${item.file}？被引用的声明会被校验拦下并自动恢复。`)) {
      return;
    }
    setSaving(true);
    try {
      await removeDeclaration(kind, item.slug);
      onSaved(`已删除 ${item.file}`);
      onClose();
    } catch (problem) {
      notify("bad", "删除失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  function applyDraft() {
    try {
      const parsed = JSON.parse(draft) as unknown;
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("声明必须是对象");
      }
      setDoc(parsed as Record<string, unknown>);
      notify("ok", "已套用到表单", "保存时会再走一次完整校验。");
    } catch (problem) {
      notify("bad", "JSON 无法解析", problem instanceof Error ? problem.message : undefined);
    }
  }

  return (
    <Drawer
      open
      kicker={editing ? `${labelOf(kind)} · ${item?.id}` : `${labelOf(kind)} · 新建`}
      title={editing ? (item?.name ?? item?.slug ?? "") : "新建声明"}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn-primary" disabled={saving} onClick={() => void submit()}>
            {saving ? "写入中…" : editing ? "保存并校验" : "创建并校验"}
          </button>
          {editing && removable ? (
            <button type="button" className="btn btn-quiet" disabled={saving} onClick={() => void discard()}>
              <IconTrash size={13} /> 删除
            </button>
          ) : null}
          {editing && !removable ? <span className="faint">内置适配器不可删除，可以停用或改写</span> : null}
          <span className="spacer" />
          <button type="button" className="btn btn-quiet btn-sm" onClick={onClose}>
            取消
          </button>
        </>
      }
    >
      <div className="form-grid">
        <Field
          label="声明名"
          hint={editing ? "文件名在新建后固定；要改名请新建一份再删除旧的" : "会成为文件名与 id，例如 arxiv-ai"}
        >
          <TextField
            value={slug}
            onChange={setSlug}
            placeholder="字母、数字、点、下划线、连字符"
            mono
            disabled={editing}
          />
        </Field>
        <Field label="启用" hint="停用后不参与采集，声明本身留在目录里">
          <button
            type="button"
            role="switch"
            aria-checked={readPath(doc, ["enabled"]) !== false}
            className={`switch${readPath(doc, ["enabled"]) !== false ? " is-on" : ""}`}
            onClick={() => update(["enabled"], readPath(doc, ["enabled"]) === false)}
          >
            <span className="switch-track" aria-hidden="true">
              <span className="switch-knob" />
            </span>
            <span className="switch-text">{readPath(doc, ["enabled"]) === false ? "停用" : "启用"}</span>
          </button>
        </Field>

        {fields.map((field) => {
          const value = readPath(doc, field.path);
          if (field.kind === "textarea") {
            return (
              <Field key={field.path.join(".")} label={field.label} hint={field.hint} wide>
                <TextArea
                  value={typeof value === "string" ? value : ""}
                  onChange={(next) => update(field.path, next)}
                  placeholder={field.placeholder}
                  rows={3}
                />
              </Field>
            );
          }
          if (field.kind === "number") {
            return (
              <Field key={field.path.join(".")} label={field.label} hint={field.hint}>
                <NumberField
                  value={typeof value === "number" ? value : null}
                  onChange={(next) => update(field.path, next)}
                  placeholder={field.placeholder}
                  min={0}
                />
              </Field>
            );
          }
          if (field.kind === "select") {
            return (
              <Field key={field.path.join(".")} label={field.label} hint={field.hint}>
                <SelectField
                  value={typeof value === "string" ? value : ""}
                  onChange={(next) => update(field.path, next)}
                  options={field.options ?? []}
                  placeholder={field.options?.length ? "未选择" : undefined}
                />
              </Field>
            );
          }
          return (
            <Field key={field.path.join(".")} label={field.label} hint={field.hint}>
              <TextField
                value={typeof value === "string" ? value : typeof value === "number" ? String(value) : ""}
                onChange={(next) => update(field.path, next)}
                placeholder={field.placeholder}
                mono
              />
            </Field>
          );
        })}

        {supportsProxy ? (
          <Field
            label="出站代理"
            hint={kind === "source" ? "覆盖这一信源的代理；留空跟随系统，采集通道页可改全局值" : "覆盖这一通道的代理"}
            wide
          >
            <ProxyInput
              value={proxyOf(doc)}
              onChange={(next) => setDoc((current) => withProxy(current, next))}
            />
          </Field>
        ) : null}
      </div>

      {editing && item && item.references > 0 ? (
        <div className="note">
          <IconAlert size={14} />
          <span>有 {item.references} 份声明引用了它；删除会被校验拦下，停用则不影响其它声明。</span>
        </div>
      ) : null}

      <div className="form-advanced">
        <button type="button" className="collapse-head" aria-expanded={advanced} onClick={() => setAdvanced(!advanced)}>
          <span className="collapse-title">高级 · 整份声明</span>
          <span className="collapse-note">字段没覆盖到的部分在这里改，保存前会重新解析</span>
        </button>
        {advanced ? (
          <div className="form-advanced-body">
            <TextArea value={draft} onChange={setDraft} rows={14} mono />
            <div className="row">
              <button type="button" className="btn btn-sm" onClick={applyDraft}>
                套用到表单
              </button>
              <button
                type="button"
                className="btn btn-sm btn-quiet"
                onClick={() => setDraft(JSON.stringify(doc, null, 2))}
              >
                用当前表单覆盖
              </button>
            </div>
            <p className="form-hint">
              `id` 与 `kind` 由系统在写入时补齐；这里只写内容字段即可。
            </p>
          </div>
        ) : null}
      </div>
    </Drawer>
  );
}

function labelOf(kind: DeclarationKind): string {
  return kind === "source" ? "信源" : kind === "accessor" ? "采集通道" : "解析器";
}
