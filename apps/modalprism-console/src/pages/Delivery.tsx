import { useCallback, useEffect, useMemo, useState } from "react";

import { IconPlus, IconRefresh, IconTrash } from "../components/icons";
import { ManualDeliveryModal } from "../components/ManualDelivery";
import { PlatformIcon } from "../components/platforms";
import { Seg, SortHeader, sortRows, useSort } from "../components/Table";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, StructuredValue, Tabs, useUi } from "../components/ui";
import {
  createDeliveryChannel,
  createDeliveryEndpoint,
  createDeliveryRule,
  deleteDeliveryChannel,
  deleteDeliveryEndpoint,
  deleteDeliveryRule,
  getDeliveryRecord,
  listDeliveryChannels,
  listDeliveryEndpoints,
  listDeliveryRecords,
  listDeliveryRules,
  listDeliveryTargets,
  listPlans,
  listSources,
  testDeliveryChannel,
  updateDeliveryChannel,
  updateDeliveryEndpoint,
  updateDeliveryRule,
} from "../lib/api";
import { relative, stamp } from "../lib/format";
import { useRoute } from "../lib/router";
import type {
  DeliveryChannel,
  DeliveryEndpoint,
  DeliveryRecord,
  DeliveryRule,
  DeliveryTarget,
  DeliveryTargetGroup,
  Plan,
  Source,
} from "../lib/types";

type Tab = "records" | "rules" | "targets";

const TYPE_LABELS: Record<string, string> = { manual: "手动投递", auto: "自动投递" };
const KIND_LABELS: Record<string, string> = { model: "模型", endpoint: "外部 API", channel: "通知渠道" };

export function DeliveryPage() {
  const { params } = useRoute();
  const { notify } = useUi();
  const [tab, setTab] = useState<Tab>((params.get("tab") as Tab) || "records");
  const [records, setRecords] = useState<DeliveryRecord[]>([]);
  const [rules, setRules] = useState<DeliveryRule[]>([]);
  const [endpoints, setEndpoints] = useState<DeliveryEndpoint[]>([]);
  const [channels, setChannels] = useState<DeliveryChannel[]>([]);
  const [targets, setTargets] = useState<DeliveryTarget[]>([]);
  const [groups, setGroups] = useState<DeliveryTargetGroup[]>([]);
  const [typeFilter, setTypeFilter] = useState<"all" | "manual" | "auto">("all");
  const [statusFilter, setStatusFilter] = useState("all");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [manual, setManual] = useState(false);
  const [ruleEditing, setRuleEditing] = useState<DeliveryRule | "new" | null>(null);
  const [endpointEditing, setEndpointEditing] = useState<DeliveryEndpoint | "new" | null>(null);
  const [channelEditing, setChannelEditing] = useState<DeliveryChannel | "new" | null>(null);
  const [detail, setDetail] = useState<string | null>(null);
  const { sort, toggle } = useSort({ key: "createdAt", direction: "desc" });

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [recordPage, rulePage, catalog, endpointPage, channelPage] = await Promise.all([
        listDeliveryRecords({ limit: 100, type: typeFilter === "all" ? undefined : typeFilter, status: statusFilter === "all" ? undefined : statusFilter }),
        listDeliveryRules({ limit: 100 }),
        listDeliveryTargets(),
        listDeliveryEndpoints(),
        listDeliveryChannels(),
      ]);
      setRecords(recordPage.items);
      setRules(rulePage.items);
      setTargets(catalog.items);
      setGroups(catalog.groups);
      setEndpoints(endpointPage.items);
      setChannels(channelPage.items);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [typeFilter, statusFilter]);

  useEffect(() => {
    void load();
  }, [load]);

  const targetName = useMemo(() => {
    const map = new Map<string, string>();
    for (const target of targets) map.set(`${target.kind}:${target.id}`, target.name ?? target.id);
    return (kind: string, id: string) => map.get(`${kind}:${id}`) ?? id;
  }, [targets]);

  const rows = useMemo(
    () =>
      sortRows(records, sort, (row, key) => {
        if (key === "createdAt") return row.createdAt;
        if (key === "type") return row.type;
        if (key === "status") return row.status;
        return "";
      }),
    [records, sort],
  );

  async function act(label: string, action: () => Promise<unknown>) {
    setBusy(label);
    try {
      await action();
      notify("ok", `${label}：完成`);
      await load();
    } catch (problem) {
      notify("bad", `${label}：失败`, problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">delivery · 内容投递</div>
          <h1 className="page-title">投递记录与规则</h1>
          <p className="page-lede">
            手动投递选内容与目标；自动投递按「信源」或「计划」订阅新增内容，计划自己的投递目标在计划表单里配。三类目标（模型、外部 API、通知渠道）都在本页维护。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => setManual(true)}>
            <IconPlus size={14} /> 手动投递
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <Tabs
        value={tab}
        onChange={setTab}
        items={[
          { id: "records", label: "投递记录", count: records.length },
          { id: "rules", label: "自动投递", count: rules.length },
          { id: "targets", label: "投递目标", count: targets.length },
        ]}
      />

      {tab === "records" ? (
        <>
          <div className="toolbar-band">
            <Seg
              value={typeFilter}
              onChange={(value) => setTypeFilter(value)}
              label="投递类型"
              options={[
                { value: "all", label: "全部" },
                { value: "manual", label: "手动投递" },
                { value: "auto", label: "自动投递" },
              ]}
            />
            <select className="select" value={statusFilter} onChange={(event) => setStatusFilter(event.target.value)} aria-label="状态筛选" style={{ width: 150 }}>
              <option value="all">全部状态</option>
              <option value="succeeded">成功</option>
              <option value="partiallySucceeded">部分成功</option>
              <option value="failed">失败</option>
              <option value="running">投递中</option>
            </select>
            <span className="fill" />
            <span className="small muted">{rows.length} 条记录</span>
          </div>
          <Card title="投递记录" sub="按类型区分手动与自动；点开看每个目标的明细" flush>
            {loading && records.length === 0 ? (
              <div className="card-body">
                <Loading lines={5} />
              </div>
            ) : rows.length === 0 ? (
              <Empty title="还没有投递记录" text="从内容页或本页发起一次手动投递，或先建一条自动投递规则。" />
            ) : (
              <div className="table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <SortHeader label="时间" sortKey="createdAt" sort={sort} onSort={toggle} />
                      <SortHeader label="类型" sortKey="type" sort={sort} onSort={toggle} />
                      <th>内容</th>
                      <th>目标</th>
                      <SortHeader label="状态" sortKey="status" sort={sort} onSort={toggle} />
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((record) => (
                      <tr key={record.id}>
                        <td className="small muted" title={stamp(record.createdAt)}>
                          {relative(record.createdAt)}
                        </td>
                        <td>
                          <span className={`badge ${record.type === "auto" ? "info" : "neutral"}`}>{TYPE_LABELS[record.type] ?? record.type}</span>
                          {record.trigger?.ruleName ? <div className="small muted">{record.trigger.ruleName}</div> : null}
                        </td>
                        <td>
                          <div className="cell-main">
                            <span className="source-name">{record.items[0]?.title ?? "—"}</span>
                            <span className="source-sub">{record.items.length > 1 ? `等 ${record.items.length} 条` : record.items[0]?.sourceName}</span>
                          </div>
                        </td>
                        <td className="small">
                          {record.targets.map((target) => targetName(target.kind, target.id)).join("、") || "—"}
                        </td>
                        <td>
                          <StatusBadge status={record.status} />
                        </td>
                        <td className="actions">
                          <button type="button" className="btn btn-sm" onClick={() => setDetail(record.id)}>
                            详情
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </Card>
        </>
      ) : null}

      {tab === "rules" ? (
        <Card
          title="自动投递"
          sub="监听信源或计划，把新增内容按目标投递；计划行里的投递目标会在保存计划时自动落成规则"
          action={
            <button type="button" className="btn btn-sm" onClick={() => setRuleEditing("new")}>
              <IconPlus size={12} /> 新建规则
            </button>
          }
          flush
        >
          {rules.length === 0 ? (
            <Empty title="还没有自动投递规则" text="例如：监听某个信源，新增内容交给模型生成摘要并提交到你的接口。" />
          ) : (
            <div className="list">
              {rules.map((rule) => (
                <div className="list-row" key={rule.id}>
                  <span className="list-main">
                    <span className="list-title">
                      {rule.name}
                      <span className="badge neutral" style={{ marginLeft: 6 }}>
                        {rule.listener.kind === "plan" ? "监听计划" : "监听信源"}
                      </span>
                      {rule.enabled ? null : <span className="badge neutral" style={{ marginLeft: 6 }}>已停用</span>}
                    </span>
                    <span className="list-meta">
                      <span>{rule.listener.name ?? rule.listener.id}</span>
                      <span>目标：{rule.targets.map((target) => targetName(target.kind, target.id)).join("、") || "—"}</span>
                      <span>最近：{rule.lastRunAt ? relative(rule.lastRunAt) : "尚未投递"}</span>
                    </span>
                  </span>
                  <button type="button" className="btn btn-sm" disabled={busy === rule.id} onClick={() => setRuleEditing(rule)}>
                    编辑
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={busy === rule.id}
                    onClick={() => void act("切换启用状态", () => updateDeliveryRule(rule.id, { enabled: !rule.enabled }, rule.etag ?? ""))}
                  >
                    {rule.enabled ? "停用" : "启用"}
                  </button>
                  <button
                    type="button"
                    className="icon-btn"
                    aria-label={`删除规则 ${rule.name}`}
                    onClick={() => {
                      if (!window.confirm(`删除规则「${rule.name}」？已有投递记录保留。`)) return;
                      void act("删除规则", () => deleteDeliveryRule(rule.id));
                    }}
                  >
                    <IconTrash size={15} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </Card>
      ) : null}

      {tab === "targets" ? (
        <div className="stack" style={{ gap: 16 }}>
          <Card
            title="外部 API"
            sub="按 JSON 提交内容；令牌只保存环境变量名"
            action={
              <button type="button" className="btn btn-sm" onClick={() => setEndpointEditing("new")}>
                <IconPlus size={12} /> 新增接口
              </button>
            }
            flush
          >
            {endpoints.length === 0 ? (
              <Empty title="还没有外部接口" text="填一个接收地址即可；提交体包含 title 与 items。" />
            ) : (
              <div className="list">
                {endpoints.map((endpoint) => (
                  <div className="list-row" key={endpoint.id}>
                    <PlatformIcon icon="endpoint" size={26} />
                    <span className="list-main">
                      <span className="list-title">
                        {endpoint.name}
                        {endpoint.enabled ? null : <span className="badge neutral" style={{ marginLeft: 6 }}>已停用</span>}
                      </span>
                      <span className="list-meta">
                        <span className="mono">{endpoint.url}</span>
                        {endpoint.tokenRef ? <span>令牌 {endpoint.tokenRef}</span> : null}
                      </span>
                    </span>
                    <button type="button" className="btn btn-sm" onClick={() => setEndpointEditing(endpoint)}>
                      编辑
                    </button>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={`删除接口 ${endpoint.name}`}
                      onClick={() => {
                        if (!window.confirm(`删除接口「${endpoint.name}」？`)) return;
                        void act("删除接口", () => deleteDeliveryEndpoint(endpoint.id));
                      }}
                    >
                      <IconTrash size={15} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </Card>

          <Card
            title="通知渠道"
            sub="投递内容到 Webhook 或飞书；地址只回传主机，密钥不出服务端"
            action={
              <button type="button" className="btn btn-sm" onClick={() => setChannelEditing("new")}>
                <IconPlus size={12} /> 新增渠道
              </button>
            }
            flush
          >
            {channels.length === 0 ? (
              <Empty title="还没有通知渠道" text="填一个 Webhook 地址或飞书机器人地址即可。" />
            ) : (
              <div className="list">
                {channels.map((channel) => (
                  <div className="list-row" key={channel.id}>
                    <span className="list-main">
                      <span className="list-title">
                        {channel.name}
                        <span className="badge neutral" style={{ marginLeft: 6 }}>
                          {channel.kind === "feishu" ? "飞书" : "Webhook"}
                        </span>
                        {channel.enabled ? null : <span className="badge neutral" style={{ marginLeft: 6 }}>已停用</span>}
                      </span>
                      <span className="list-meta">
                        <span className="mono">{String(channel.settings?.url ?? "—")}</span>
                      </span>
                    </span>
                    <button
                      type="button"
                      className="btn btn-ghost btn-sm"
                      disabled={busy === "连通检测"}
                      onClick={() => void act("连通检测", () => testDeliveryChannel(channel.id))}
                    >
                      检测
                    </button>
                    <button type="button" className="btn btn-sm" onClick={() => setChannelEditing(channel)}>
                      编辑
                    </button>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={`删除渠道 ${channel.name}`}
                      onClick={() => {
                        if (!window.confirm(`删除渠道「${channel.name}」？已发出的投递记录保留。`)) return;
                        void act("删除渠道", () => deleteDeliveryChannel(channel.id, channel.etag ?? ""));
                      }}
                    >
                      <IconTrash size={15} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </Card>

          {groups.map((group) => {
            if (group.kind === "endpoint" || group.kind === "channel") return null;
            const items = targets.filter((target) => target.kind === group.kind);
            return (
              <Card key={group.kind} title={group.label} sub={group.note} flush>
                {items.length === 0 ? (
                  <Empty title="暂无可用目标" text="先到系统设置登记模型。" />
                ) : (
                  <div className="list">
                    {items.map((target) => (
                      <div className="list-row" key={`${target.kind}:${target.id}`}>
                        <PlatformIcon icon={target.kind} size={26} />
                        <span className="list-main">
                          <span className="list-title">{target.name ?? target.id}</span>
                          <span className="list-meta">
                            <span>{target.detail ?? target.kind}</span>
                            <span>{target.enabled === false ? "已停用" : "可用"}</span>
                          </span>
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </Card>
            );
          })}
        </div>
      ) : null}

      {manual ? (
        <ManualDeliveryModal
          onClose={() => setManual(false)}
          onDone={() => {
            setTab("records");
            void load();
          }}
        />
      ) : null}
      {ruleEditing ? (
        <RuleModal
          rule={ruleEditing === "new" ? null : ruleEditing}
          targets={targets}
          groups={groups}
          onClose={() => setRuleEditing(null)}
          onDone={() => {
            setRuleEditing(null);
            void load();
          }}
        />
      ) : null}
      {endpointEditing ? (
        <EndpointModal
          endpoint={endpointEditing === "new" ? null : endpointEditing}
          onClose={() => setEndpointEditing(null)}
          onDone={() => {
            setEndpointEditing(null);
            void load();
          }}
        />
      ) : null}
      {channelEditing ? (
        <ChannelModal
          channel={channelEditing === "new" ? null : channelEditing}
          onClose={() => setChannelEditing(null)}
          onDone={() => {
            setChannelEditing(null);
            void load();
          }}
        />
      ) : null}
      {detail ? <RecordDrawer id={detail} onClose={() => setDetail(null)} /> : null}
    </div>
  );
}

function RecordDrawer({ id, onClose }: { id: string; onClose: () => void }) {
  const [record, setRecord] = useState<DeliveryRecord | null>(null);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    let alive = true;
    void getDeliveryRecord(id)
      .then((value) => {
        if (alive) setRecord(value);
      })
      .catch((problem) => {
        if (alive) setError(problem);
      });
    return () => {
      alive = false;
    };
  }, [id]);

  return (
    <Drawer title={record?.items[0]?.title ?? "投递详情"} sub={record ? `${TYPE_LABELS[record.type] ?? record.type} · ${stamp(record.createdAt)}` : id} onClose={onClose} wide footer={<button type="button" className="btn" onClick={onClose}>关闭</button>}>
      {error ? <Problem error={error} /> : null}
      {!record ? (
        <Loading lines={5} />
      ) : (
        <div className="stack" style={{ gap: 16 }}>
          <div className="row wrap" style={{ gap: 8 }}>
            <StatusBadge status={record.status} />
            {record.note ? <span className="small muted">备注：{record.note}</span> : null}
          </div>
          <Card title="内容" flush>
            <div className="list">
              {record.items.map((item) => (
                <div className="list-row" key={item.contentId}>
                  <span className="list-main">
                    <span className="list-title">{item.title}</span>
                    <span className="list-meta">
                      <span>{item.sourceName}</span>
                      <span className="mono">{item.contentId}</span>
                    </span>
                  </span>
                </div>
              ))}
            </div>
          </Card>
          <Card title="目标结果" flush>
            <div className="list">
              {record.results.map((result) => (
                <div className="list-row" key={`${result.targetKind}:${result.targetId}`}>
                  <span className="list-main">
                    <span className="list-title">
                      {result.targetName ?? result.targetId}
                      <span className="badge neutral" style={{ marginLeft: 6 }}>{KIND_LABELS[result.targetKind] ?? result.targetKind}</span>
                    </span>
                    <span className="list-meta">
                      <span>{result.status === "failed" ? "失败" : "成功"}</span>
                      {typeof result.latencyMs === "number" ? <span>{result.latencyMs}ms</span> : null}
                    </span>
                  </span>
                  {result.error ? <span className="small" style={{ color: "var(--danger)" }}>{result.error}</span> : null}
                  {result.output ? (
                    <details className="small">
                      <summary>查看输出</summary>
                      <div style={{ marginTop: 6 }}>
                        <StructuredValue value={result.output} />
                      </div>
                    </details>
                  ) : null}
                </div>
              ))}
            </div>
          </Card>
        </div>
      )}
    </Drawer>
  );
}

function RuleModal({
  rule,
  targets,
  groups,
  onClose,
  onDone,
}: {
  rule: DeliveryRule | null;
  targets: DeliveryTarget[];
  groups: DeliveryTargetGroup[];
  onClose: () => void;
  onDone: () => void;
}) {
  const { notify } = useUi();
  const [name, setName] = useState(rule?.name ?? "");
  const [listenerKind, setListenerKind] = useState<"source" | "plan">(rule?.listener.kind ?? "source");
  const [listenerId, setListenerId] = useState(rule?.listener.id ?? "");
  const [chosen, setChosen] = useState<string[]>((rule?.targets ?? []).map((target) => `${target.kind}:${target.id}`));
  const [enabled, setEnabled] = useState(rule?.enabled ?? true);
  const [sources, setSources] = useState<Source[]>([]);
  const [plans, setPlans] = useState<Plan[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    void (async () => {
      try {
        const [sourcePage, planPage] = await Promise.all([listSources({ limit: 200 }), listPlans({ limit: 200 })]);
        setSources(sourcePage.items);
        setPlans(planPage.items);
      } catch (problem) {
        setError(problem);
      }
    })();
  }, []);

  async function save() {
    if (!listenerId) {
      notify("warn", "先选择要监听的信源或计划");
      return;
    }
    if (!chosen.length) {
      notify("warn", "至少选择一个投递目标");
      return;
    }
    setBusy(true);
    try {
      const payload = {
        name: name.trim() || "自动投递规则",
        listener: { kind: listenerKind, id: listenerId },
        targets: chosen.map((value) => {
          const at = value.indexOf(":");
          return { kind: value.slice(0, at), id: value.slice(at + 1) };
        }),
        enabled,
      };
      if (rule) await updateDeliveryRule(rule.id, payload, rule.etag ?? "");
      else await createDeliveryRule(payload);
      notify("ok", rule ? "规则已更新" : "规则已创建");
      onDone();
    } catch (problem) {
      notify("bad", "保存规则失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={rule ? "编辑自动投递规则" : "新建自动投递规则"}
      sub="监听对象的新增内容会按目标投递；计划按它包含的信源计算"
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void save()}>
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 14 }}>
        {error ? <Problem error={error} /> : null}
        <Field label="规则名称">
          <input className="input" value={name} onChange={(event) => setName(event.target.value)} placeholder="例如：每日信源摘要投递" />
        </Field>
        <div className="grid cols-2">
          <Field label="监听对象">
            <select
              className="select"
              value={listenerKind}
              onChange={(event) => {
                setListenerKind(event.target.value as "source" | "plan");
                setListenerId("");
              }}
            >
              <option value="source">信源</option>
              <option value="plan">计划</option>
            </select>
          </Field>
          <Field label={listenerKind === "source" ? "选择信源" : "选择计划"}>
            <select className="select" value={listenerId} onChange={(event) => setListenerId(event.target.value)}>
              <option value="">请选择</option>
              {(listenerKind === "source" ? sources : plans).map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name ?? item.id}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label="投递目标（可多选）">
          <div className="stack" style={{ gap: 6 }}>
            {groups.map((group) => {
              const items = targets.filter((target) => target.kind === group.kind && target.enabled !== false);
              if (!items.length) return null;
              return (
                <div key={group.kind} className="stack" style={{ gap: 4 }}>
                  <span className="small muted">{group.label}</span>
                  <div className="row wrap" style={{ gap: 10 }}>
                    {items.map((target) => {
                      const key = `${target.kind}:${target.id}`;
                      return (
                        <label key={key} className="row" style={{ gap: 5 }}>
                          <input
                            type="checkbox"
                            checked={chosen.includes(key)}
                            onChange={() =>
                              setChosen((current) => (current.includes(key) ? current.filter((item) => item !== key) : [...current, key]))
                            }
                          />
                          <span className="small">{target.name ?? target.id}</span>
                        </label>
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </div>
        </Field>
        <label className="switch">
          <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
          <span>启用这条规则</span>
        </label>
      </div>
    </Modal>
  );
}

function EndpointModal({ endpoint, onClose, onDone }: { endpoint: DeliveryEndpoint | null; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [name, setName] = useState(endpoint?.name ?? "");
  const [url, setUrl] = useState(endpoint?.url ?? "");
  const [tokenRef, setTokenRef] = useState(endpoint?.tokenRef ?? "");
  const [enabled, setEnabled] = useState(endpoint?.enabled ?? true);
  const [busy, setBusy] = useState(false);

  async function save() {
    setBusy(true);
    try {
      const payload = { name, url: url.trim(), tokenRef: tokenRef.trim(), enabled };
      if (endpoint) await updateDeliveryEndpoint(endpoint.id, payload, endpoint.etag ?? "");
      else await createDeliveryEndpoint(payload);
      notify("ok", endpoint ? "接口已更新" : "接口已创建");
      onDone();
    } catch (problem) {
      notify("bad", "保存失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={endpoint ? "编辑外部接口" : "新增外部接口"}
      sub="提交体为 JSON：title、kind、items；令牌只保存环境变量名"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button type="button" className="btn btn-primary" disabled={busy || !name.trim() || !url.trim()} onClick={() => void save()}>
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="名称">
          <input className="input" value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
        <Field label="接收地址">
          <input className="input mono" value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://example.com/hooks/content" />
        </Field>
        <Field label="令牌环境变量名" hint="可选；存在时按 Bearer 发送">
          <input className="input mono" value={tokenRef} onChange={(event) => setTokenRef(event.target.value)} placeholder="MY_WEBHOOK_TOKEN" />
        </Field>
        <label className="switch">
          <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
          <span>启用这个接口</span>
        </label>
      </div>
    </Modal>
  );
}

/** 通知渠道：webhook 或飞书机器人。地址只回传主机，编辑时回传掩码表示不改地址。 */
function ChannelModal({
  channel,
  onClose,
  onDone,
}: {
  channel: DeliveryChannel | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const { notify } = useUi();
  const [name, setName] = useState(channel?.name ?? "");
  const [kind, setKind] = useState<"webhook" | "feishu">(channel?.kind === "feishu" ? "feishu" : "webhook");
  const [url, setUrl] = useState(String(channel?.settings?.url ?? ""));
  const [enabled, setEnabled] = useState(channel?.enabled ?? true);
  const [busy, setBusy] = useState(false);

  async function save() {
    setBusy(true);
    try {
      const payload = { name: name.trim(), kind, enabled, settings: { url: url.trim() } };
      if (channel) await updateDeliveryChannel(channel.id, payload, channel.etag ?? "");
      else await createDeliveryChannel(payload);
      notify("ok", channel ? "渠道已更新" : "渠道已创建");
      onDone();
    } catch (problem) {
      notify("bad", "保存失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={channel ? "编辑通知渠道" : "新增通知渠道"}
      sub="Webhook 按 JSON 提交；飞书用机器人地址。令牌密钥不进备份、不回传。"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button type="button" className="btn btn-primary" disabled={busy || !name.trim() || !url.trim()} onClick={() => void save()}>
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="名称">
          <input className="input" value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
        <Field label="类型">
          <select className="select" value={kind} onChange={(event) => setKind(event.target.value as "webhook" | "feishu")}>
            <option value="webhook">Webhook</option>
            <option value="feishu">飞书机器人</option>
          </select>
        </Field>
        <Field label="投递地址" hint="保存后只回传主机，路径与密钥留在服务端">
          <input
            className="input mono"
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            placeholder={kind === "feishu" ? "https://open.feishu.cn/open-apis/bot/v2/hook/***" : "https://example.com/hooks/content"}
          />
        </Field>
        <label className="switch">
          <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
          <span>启用这个渠道</span>
        </label>
      </div>
    </Modal>
  );
}
