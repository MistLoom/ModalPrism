import { useCallback, useEffect, useState } from "react";

import { IconCheck, IconPlus, IconRefresh, IconTrash } from "../components/icons";
import { Card, Empty, Field, Loading, Modal, Problem, StatusBadge, Tabs, useUi } from "../components/ui";
import {
  createChannel,
  deleteChannel,
  egressStatus,
  listChannels,
  listDeliveries,
  listPublications,
  resetEgressHost,
  retryDelivery,
  testNotification,
  updateChannel,
} from "../lib/api";
import { relative, stamp, statusLabel } from "../lib/format";
import { useRoute } from "../lib/router";
import type { Channel, Delivery, Operation, Publication } from "../lib/types";

type Tab = "channels" | "publications" | "deliveries" | "egress";

export function OutboundPage() {
  const { params } = useRoute();
  const { notify, run } = useUi();
  const [tab, setTab] = useState<Tab>((params.get("tab") as Tab) || "channels");
  const [channels, setChannels] = useState<Channel[]>([]);
  const [publications, setPublications] = useState<Publication[]>([]);
  const [deliveries, setDeliveries] = useState<Delivery[]>([]);
  const [egress, setEgress] = useState<Record<string, unknown> | null>(null);
  const [status, setStatus] = useState(params.get("status") ?? "all");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<Channel | "new" | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [channelPage, publicationPage, deliveryPage, egressValue] = await Promise.all([
        listChannels(),
        listPublications({ limit: 100, status: status === "all" ? undefined : status }),
        listDeliveries({ limit: 100, status: status === "all" ? undefined : status }),
        egressStatus(),
      ]);
      setChannels(channelPage.items);
      setPublications(publicationPage.items);
      setDeliveries(deliveryPage.items);
      setEgress(egressValue);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [status]);

  useEffect(() => {
    void load();
  }, [load]);

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

  // 异步动作由 run 统一提示结果；这里只负责创建请求失败的提示与忙碌态。
  async function perform(label: string, action: () => Promise<Operation>) {
    setBusy(label);
    let started = false;
    try {
      const operation = await action();
      started = true;
      await run(operation, label);
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", `${label}：失败`, problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">outbound · 出站</div>
          <h1 className="page-title">渠道、发布与投递治理</h1>
          <p className="page-lede">
            发布意图与单渠道投递分别持久化；一个渠道失败不会吞掉其他渠道的结果。通知测试走正式的出站与脱敏路径，但不会创建发布记录。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => setEditing("new")}>
            <IconPlus size={14} /> 新建渠道
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <Tabs
        value={tab}
        onChange={setTab}
        items={[
          { id: "channels", label: "渠道", count: channels.length },
          { id: "publications", label: "发布", count: publications.length },
          { id: "deliveries", label: "投递", count: deliveries.length },
          { id: "egress", label: "出站治理" },
        ]}
      />

      {tab !== "channels" && tab !== "egress" ? (
        <div className="toolbar-band">
          <div className="seg" role="group" aria-label="状态筛选">
            {["all", "queued", "published", "partiallyPublished", "failed", "delivered"].map((value) => (
              <button key={value} type="button" className={status === value ? "is-active" : ""} onClick={() => setStatus(value)}>
                {value === "all" ? "全部" : statusLabel(value)}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      {tab === "channels" ? (
        loading && channels.length === 0 ? <Loading lines={4} /> : channels.length === 0 ? (
          <Card title="渠道"><Empty title="还没有投递渠道" text="添加一个 Webhook 或飞书群机器人。" action={<button type="button" className="btn btn-primary btn-sm" onClick={() => setEditing("new")}>新建渠道</button>} /></Card>
        ) : (
          <div className="grid cols-3">
            {channels.map((channel) => (
              <div className="channel-card" key={channel.id}>
                <div className="row between">
                  <strong>{channel.name}</strong>
                  <span className={`badge ${channel.enabled ? "ok" : "neutral"}`}>{channel.enabled ? "启用" : "停用"}</span>
                </div>
                <div className="small muted mono">{channel.kind} · {String(channel.settings.url ?? "未配置地址")}</div>
                <div className="row wrap">
                  <button type="button" className="btn btn-sm" onClick={() => setEditing(channel)}>编辑</button>
                  <button type="button" className="btn btn-sm" disabled={busy === `test-${channel.id}`} onClick={() => void perform("通知测试", async () => testNotification(channel.id))}>
                    <IconCheck size={12} /> 试发
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm btn-danger"
                    onClick={() => {
                      if (!window.confirm(`删除渠道「${channel.name}」？已发布的投递记录会保留。`)) return;
                      void act("删除渠道", async () => {
                        await deleteChannel(channel.id, channel.etag ?? "");
                      });
                    }}
                  >
                    <IconTrash size={12} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )
      ) : null}

      {tab === "publications" ? (
        <Card title="发布" sub="Publication 状态由 Delivery 汇总" flush>
          {publications.length === 0 ? <Empty title="还没有发布" text="在生产页创建草稿后发起发布。" /> : (
            <div className="table-wrap">
              <table className="table">
                <thead><tr><th>标题</th><th>状态</th><th>渠道</th><th>创建</th><th /></tr></thead>
                <tbody>
                  {publications.map((publication) => (
                    <tr key={publication.id}>
                      <td><div className="cell-main"><span className="source-name">{publication.title}</span><span className="source-sub mono">{publication.id}</span></div></td>
                      <td><StatusBadge status={publication.status} /></td>
                      <td className="small">{publication.channels.map((channel) => channel.name).join("、")}</td>
                      <td className="small muted">{relative(publication.createdAt)}</td>
                      <td className="actions">
                        <button type="button" className="btn btn-sm" onClick={() => setTab("deliveries")}>投递</button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Card>
      ) : null}

      {tab === "deliveries" ? (
        <Card title="投递" sub="失败保留错误类别与远端关联号" flush>
          {deliveries.length === 0 ? <Empty title="还没有投递" text="发布创建后每个渠道各有一条 Delivery。" /> : (
            <div className="table-wrap">
              <table className="table">
                <thead><tr><th>渠道</th><th>状态</th><th>尝试</th><th>错误</th><th>创建</th><th /></tr></thead>
                <tbody>
                  {deliveries.map((delivery) => (
                    <tr key={delivery.id}>
                      <td><div className="cell-main"><span className="source-name">{delivery.channelName || delivery.channelId}</span><span className="source-sub mono">{delivery.id}</span></div></td>
                      <td><StatusBadge status={delivery.status} /></td>
                      <td className="mono">{delivery.attempts.length}</td>
                      <td className="small" style={{ color: delivery.lastError ? "var(--danger)" : undefined }}>{delivery.lastError || "—"}</td>
                      <td className="small muted">{stamp(delivery.createdAt)}</td>
                      <td className="actions">
                        <button type="button" className="btn btn-sm" disabled={busy === `retry-${delivery.id}`} onClick={() => void perform("重试投递", async () => retryDelivery(delivery.id, "控制台重试"))}>
                          重试
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Card>
      ) : null}

      {tab === "egress" ? (
        <div className="split-2">
          <Card title="出站状态" sub="脱敏后的治理投影；不返回代理凭据" flush>
            {egress ? <div className="card-body"><div className="code">{JSON.stringify(egress, null, 2)}</div></div> : <div className="card-body"><Loading lines={4} /></div>}
          </Card>
          <Card title="主机复位" sub="清除熔断与退避状态，不影响历史审计">
            <HostReset onDone={() => void load()} />
          </Card>
        </div>
      ) : null}

      {editing ? <ChannelModal channel={editing === "new" ? null : editing} onClose={() => setEditing(null)} onDone={() => { setEditing(null); void load(); }} /> : null}
    </div>
  );
}

function HostReset({ onDone }: { onDone: () => void }) {
  const { notify, run } = useUi();
  const [host, setHost] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <div className="stack" style={{ gap: 10 }}>
      <Field label="主机" hint="例如 api.openai.com">
        <input className="input mono" value={host} onChange={(event) => setHost(event.target.value)} />
      </Field>
      <button type="button" className="btn btn-primary" disabled={busy || !host.trim()} onClick={() => void (async () => {
        setBusy(true);
        let started = false;
        try {
          const operation = await resetEgressHost(host.trim());
          started = true;
          await run(operation, `复位 ${host.trim()}`);
          onDone();
        } catch (problem) {
          if (!started) {
            notify("bad", "复位失败", problem instanceof Error ? problem.message : undefined);
          }
        } finally {
          setBusy(false);
        }
      })()}>{busy ? "复位中…" : "复位主机"}</button>
      <div className="small muted">复位会清除失败计数与暂停截止时间；限速、robots 与 SSRF 策略不会因此放宽。</div>
    </div>
  );
}

function ChannelModal({ channel, onClose, onDone }: { channel: Channel | null; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [name, setName] = useState(channel?.name ?? "");
  const [kind, setKind] = useState(channel?.kind ?? "webhook");
  const [enabled, setEnabled] = useState(channel?.enabled ?? true);
  const [url, setUrl] = useState(String(channel?.settings.url ?? ""));
  const [busy, setBusy] = useState(false);

  async function save() {
    setBusy(true);
    try {
      const payload = { name, kind, enabled, settings: { url } };
      if (channel) {
        await updateChannel(channel.id, payload, channel.etag ?? "");
      } else {
        await createChannel(payload);
      }
      notify("ok", "渠道已保存");
      onDone();
    } catch (problem) {
      notify("bad", "保存渠道失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={channel ? "编辑渠道" : "新建渠道"}
      onClose={onClose}
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !name.trim()} onClick={() => void save()}>{busy ? "保存中…" : "保存"}</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="名称"><input className="input" value={name} onChange={(event) => setName(event.target.value)} /></Field>
        <Field label="类型">
          <select className="select" value={kind} onChange={(event) => setKind(event.target.value)}>
            <option value="webhook">通用 Webhook</option>
            <option value="feishu">飞书群机器人</option>
          </select>
        </Field>
        <Field label="投递地址" hint="已保存的地址仅显示主机；保持 *** 掩码即不修改密钥，填入完整地址则替换">
          <input className="input mono" value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://…" />
        </Field>
        <label className="switch"><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} /><span>启用渠道</span></label>
      </div>
    </Modal>
  );
}
