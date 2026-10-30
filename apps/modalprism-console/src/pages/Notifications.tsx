import { useCallback, useEffect, useMemo, useState } from "react";

import { IconCheck, IconPlus, IconRefresh, IconTrash } from "../components/icons";
import { PlatformIcon } from "../components/platforms";
import { Seg, SortHeader, sortRows, useSort } from "../components/Table";
import { Card, Empty, Field, Loading, Modal, Problem, StatusBadge, Tabs, useUi } from "../components/ui";
import {
  createPushChannel,
  deletePushChannel,
  getNotificationCatalog,
  getNotificationSettings,
  listNotificationCenter,
  listPushChannels,
  listPushPlatforms,
  listPushRecords,
  markNotificationsRead,
  testPush,
  updateNotificationSettings,
  updatePushChannel,
} from "../lib/api";
import { relative, stamp } from "../lib/format";
import { useRoute } from "../lib/router";
import type {
  NotificationCenterSettings,
  NotificationEntry,
  PushChannel,
  PushPlatform,
  PushRecord,
  PushTestResult,
} from "../lib/types";

type Tab = "center" | "push" | "records";

const CATEGORY_TONE: Record<string, string> = {
  plan: "info",
  source: "neutral",
  content: "neutral",
  delivery: "ok",
  system: "warn",
};

export function NotificationsPage() {
  const { params } = useRoute();
  const { notify } = useUi();
  const [tab, setTab] = useState<Tab>((params.get("tab") as Tab) || "center");
  const [entries, setEntries] = useState<NotificationEntry[]>([]);
  const [unread, setUnread] = useState(0);
  const [category, setCategory] = useState("all");
  const [categories, setCategories] = useState<{ id: string; label: string }[]>([]);
  const [settings, setSettings] = useState<NotificationCenterSettings | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [platforms, setPlatforms] = useState<PushPlatform[]>([]);
  const [events, setEvents] = useState<{ id: string; label: string }[]>([]);
  const [channels, setChannels] = useState<PushChannel[]>([]);
  const [records, setRecords] = useState<PushRecord[]>([]);
  const [recordStatus, setRecordStatus] = useState("all");
  const [channelEditing, setChannelEditing] = useState<PushChannel | "new" | null>(null);
  const [presetPlatform, setPresetPlatform] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const { sort, toggle } = useSort({ key: "createdAt", direction: "desc" });

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [center, catalog, currentSettings, platformPage, channelPage, recordPage] = await Promise.all([
        listNotificationCenter({ limit: 100, category: category === "all" ? undefined : category }),
        getNotificationCatalog(),
        getNotificationSettings(),
        listPushPlatforms(),
        listPushChannels(),
        listPushRecords({ limit: 100, status: recordStatus === "all" ? undefined : recordStatus }),
      ]);
      setEntries(center.items);
      setUnread(center.unread);
      setCategories(catalog.categories);
      setSettings(currentSettings);
      setPlatforms(platformPage.items);
      setEvents(platformPage.events);
      setChannels(channelPage.items);
      setRecords(recordPage.items);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [category, recordStatus]);

  useEffect(() => {
    void load();
  }, [load]);

  const eventLabel = useMemo(() => {
    const map = new Map(events.map((item) => [item.id, item.label]));
    return (id: string) => map.get(id) ?? id;
  }, [events]);

  const platformOf = useMemo(() => {
    const map = new Map(platforms.map((item) => [item.id, item]));
    return (id: string) => map.get(id);
  }, [platforms]);

  const sortedRecords = useMemo(
    () =>
      sortRows(records, sort, (row, key) => {
        if (key === "createdAt") return row.createdAt;
        if (key === "status") return row.status;
        if (key === "channelName") return row.channelName;
        return "";
      }),
    [records, sort],
  );

  async function act(label: string, action: () => Promise<unknown>) {
    setBusy(label);
    try {
      await action();
      await load();
    } catch (problem) {
      notify("bad", `${label}：失败`, problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  async function saveSettings(patch: Partial<NotificationCenterSettings>) {
    try {
      setSettings(await updateNotificationSettings(patch));
      notify("ok", "通知设置已保存");
    } catch (problem) {
      notify("bad", "保存通知设置失败", problem instanceof Error ? problem.message : undefined);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">notifications · 通知管理</div>
          <h1 className="page-title">通知中心与通知推送</h1>
          <p className="page-lede">
            通知中心把系统事件留在本机（浏览器提醒），通知推送把它们发到你常用的平台。两者只看事件分类，不混入内容投递。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => { setPresetPlatform(""); setChannelEditing("new"); }}>
            <IconPlus size={14} /> 添加推送渠道
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <Tabs
        value={tab}
        onChange={setTab}
        items={[
          { id: "center", label: "通知中心", count: unread || undefined },
          { id: "push", label: "通知推送", count: channels.length },
          { id: "records", label: "推送记录", count: records.length },
        ]}
      />

      {tab === "center" ? (
        <>
          <div className="toolbar-band">
            <Seg
              value={category}
              onChange={setCategory}
              label="通知分类"
              options={[{ value: "all", label: "全部" }, ...categories.map((item) => ({ value: item.id, label: item.label }))]}
            />
            <span className="fill" />
            <button
              type="button"
              className="btn btn-sm"
              disabled={!unread}
              onClick={() => void act("全部标为已读", () => markNotificationsRead({ all: true }))}
            >
              <IconCheck size={12} /> 全部已读
            </button>
            <button type="button" className="btn btn-sm" onClick={() => setSettingsOpen(true)}>
              通知设置
            </button>
          </div>
          <Card title="最近通知" sub="只保留最近 500 条；分类开关关闭后不再记录该类事件" flush>
            {loading && entries.length === 0 ? (
              <div className="card-body">
                <Loading lines={5} />
              </div>
            ) : entries.length === 0 ? (
              <Empty title="还没有通知" text="采集、投递与系统事件会按分类开关产生通知。" />
            ) : (
              <div className="list">
                {entries.map((entry) => (
                  <div className={`list-row${entry.read ? "" : " is-unread"}`} key={entry.id}>
                    <span className={`dot ${entry.level === "warn" ? "warn" : "ok"}`} />
                    <span className="list-main">
                      <span className="list-title">{entry.title}</span>
                      <span className="list-meta">
                        <span>{entry.message}</span>
                        <span>{relative(entry.createdAt)}</span>
                      </span>
                    </span>
                    <span className={`badge ${CATEGORY_TONE[entry.category] ?? "neutral"}`}>
                      {categories.find((item) => item.id === entry.category)?.label ?? entry.category}
                    </span>
                    {entry.read ? null : (
                      <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        onClick={() => void act("标为已读", () => markNotificationsRead({ ids: [entry.id] }))}
                      >
                        标为已读
                      </button>
                    )}
                  </div>
                ))}
              </div>
            )}
          </Card>
        </>
      ) : null}

      {tab === "push" ? (
        <div className="stack" style={{ gap: 16 }}>
          <div className="platform-grid">
            {platforms.map((platform) => {
              const configured = channels.filter((channel) => channel.platform === platform.id).length;
              return (
                <button
                  type="button"
                  key={platform.id}
                  className="platform-card"
                  onClick={() => {
                    setPresetPlatform(platform.id);
                    setChannelEditing("new");
                  }}
                >
                  <PlatformIcon icon={platform.icon} label={platform.name} size={38} />
                  <span className="platform-card-main">
                    <span className="platform-card-title">{platform.name}</span>
                    <span className="platform-card-note">{platform.note}</span>
                  </span>
                  <span className="small muted">{configured ? `${configured} 个渠道` : "未配置"}</span>
                </button>
              );
            })}
          </div>

          <Card title="已配置的推送渠道" sub="勾选需要推送的事件；未勾选的事件不会发送" flush>
            {channels.length === 0 ? (
              <Empty title="还没有推送渠道" text="点上方任一平台，填必要设置即可。" />
            ) : (
              <div className="list">
                {channels.map((channel) => (
                  <div className="list-row" key={channel.id}>
                    <PlatformIcon icon={platformOf(channel.platform)?.icon ?? "webhook"} label={platformOf(channel.platform)?.name} size={26} />
                    <span className="list-main">
                      <span className="list-title">
                        {channel.name}
                        {channel.enabled ? null : <span className="badge neutral" style={{ marginLeft: 6 }}>已停用</span>}
                      </span>
                      <span className="list-meta">
                        <span>{platformOf(channel.platform)?.name ?? channel.platform}</span>
                        <span>{channel.events.length ? channel.events.map(eventLabel).join("、") : "未选择事件"}</span>
                      </span>
                    </span>
                    <button
                      type="button"
                      className="btn btn-sm"
                      disabled={busy === `test-${channel.id}`}
                      onClick={() =>
                        void (async () => {
                          setBusy(`test-${channel.id}`);
                          try {
                            const result = await testPush({ channelId: channel.id });
                            notify(result.ok ? "ok" : "warn", result.ok ? "连通正常" : "连通失败", `${result.latencyMs}ms · ${result.detail}`);
                          } catch (problem) {
                            notify("bad", "连通检测失败", problem instanceof Error ? problem.message : undefined);
                          } finally {
                            setBusy(null);
                          }
                        })()
                      }
                    >
                      测试
                    </button>
                    <button type="button" className="btn btn-sm" onClick={() => setChannelEditing(channel)}>
                      编辑
                    </button>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={`删除渠道 ${channel.name}`}
                      onClick={() => {
                        if (!window.confirm(`删除推送渠道「${channel.name}」？`)) return;
                        void act("删除渠道", () => deletePushChannel(channel.id));
                      }}
                    >
                      <IconTrash size={15} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </Card>
        </div>
      ) : null}

      {tab === "records" ? (
        <>
          <div className="toolbar-band">
            <Seg
              value={recordStatus}
              onChange={setRecordStatus}
              label="推送状态"
              options={[
                { value: "all", label: "全部" },
                { value: "sent", label: "已发送" },
                { value: "failed", label: "失败" },
              ]}
            />
            <span className="fill" />
            <span className="small muted">{sortedRecords.length} 条</span>
          </div>
          <Card title="推送记录" sub="失败保留平台返回的错误摘要" flush>
            {sortedRecords.length === 0 ? (
              <Empty title="还没有推送记录" text="事件触发后按渠道订阅产生记录；也可用渠道上的「测试」验证连通。" />
            ) : (
              <div className="table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <SortHeader label="时间" sortKey="createdAt" sort={sort} onSort={toggle} />
                      <SortHeader label="渠道" sortKey="channelName" sort={sort} onSort={toggle} />
                      <th>事件</th>
                      <SortHeader label="状态" sortKey="status" sort={sort} onSort={toggle} />
                      <th>说明</th>
                    </tr>
                  </thead>
                  <tbody>
                    {sortedRecords.map((record) => (
                      <tr key={record.id}>
                        <td className="small muted" title={stamp(record.createdAt)}>
                          {relative(record.createdAt)}
                        </td>
                        <td className="small">
                          <div className="row" style={{ gap: 6 }}>
                            <PlatformIcon icon={platformOf(record.platform)?.icon ?? "webhook"} size={20} />
                            {record.channelName}
                          </div>
                        </td>
                        <td className="small">{eventLabel(record.event)}</td>
                        <td>
                          <StatusBadge status={record.status === "sent" ? "succeeded" : record.status} />
                        </td>
                        <td className="small" style={{ color: record.error ? "var(--danger)" : undefined }}>
                          {record.error || record.title}
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

      {settingsOpen && settings ? (
        <Modal
          title="通知设置"
          sub="分类开关决定是否记录与提醒；浏览器提醒需要页面处于打开状态"
          onClose={() => setSettingsOpen(false)}
          footer={
            <button type="button" className="btn btn-primary" onClick={() => setSettingsOpen(false)}>
              完成
            </button>
          }
        >
          <div className="stack" style={{ gap: 12 }}>
            <label className="switch">
              <input type="checkbox" checked={settings.enabled} onChange={(event) => void saveSettings({ enabled: event.target.checked })} />
              <span>启用通知中心</span>
            </label>
            <label className="switch">
              <input type="checkbox" checked={settings.desktop} onChange={(event) => void saveSettings({ desktop: event.target.checked })} />
              <span>在工作台弹出浏览器提醒</span>
            </label>
            <div className="stack" style={{ gap: 6 }}>
              <span className="field-label">通知分类</span>
              {categories.map((item) => (
                <label className="switch" key={item.id} style={{ justifyContent: "space-between" }}>
                  <span>{item.label}</span>
                  <input
                    type="checkbox"
                    checked={settings.categories[item.id] ?? true}
                    onChange={(event) => void saveSettings({ categories: { [item.id]: event.target.checked } })}
                  />
                </label>
              ))}
            </div>
          </div>
        </Modal>
      ) : null}

      {channelEditing ? (
        <ChannelModal
          channel={channelEditing === "new" ? null : channelEditing}
          platforms={platforms}
          events={events}
          presetPlatform={presetPlatform}
          onClose={() => setChannelEditing(null)}
          onDone={() => {
            setChannelEditing(null);
            void load();
          }}
        />
      ) : null}
    </div>
  );
}

function ChannelModal({
  channel,
  platforms,
  events,
  presetPlatform,
  onClose,
  onDone,
}: {
  channel: PushChannel | null;
  platforms: PushPlatform[];
  events: { id: string; label: string }[];
  presetPlatform: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const { notify } = useUi();
  const initialPlatform = channel?.platform ?? presetPlatform ?? "";
  const [platformId, setPlatformId] = useState(initialPlatform);
  const [name, setName] = useState(channel?.name ?? "");
  const [settings, setSettings] = useState<Record<string, string>>(channel?.settings ?? {});
  const [subscribed, setSubscribed] = useState<string[]>(channel?.events ?? ["collection.completed", "content.delivered"]);
  const [enabled, setEnabled] = useState(channel?.enabled ?? true);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<PushTestResult | null>(null);
  const platform = platforms.find((item) => item.id === platformId);

  function changePlatform(id: string) {
    setPlatformId(id);
    setSettings({});
    setResult(null);
    const found = platforms.find((item) => item.id === id);
    if (!name.trim() && found) setName(found.name);
  }

  async function test() {
    if (!platform) return;
    setTesting(true);
    setResult(null);
    try {
      setResult(await testPush({ platform: platform.id, settings }));
    } catch (problem) {
      notify("bad", "连通检测失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setTesting(false);
    }
  }

  async function save() {
    if (!platform) {
      notify("warn", "先选择平台");
      return;
    }
    setBusy(true);
    try {
      const payload = {
        name: name.trim() || platform.name,
        platform: platform.id,
        settings,
        events: subscribed,
        enabled,
      };
      if (channel) await updatePushChannel(channel.id, payload, channel.etag ?? "");
      else await createPushChannel(payload);
      notify("ok", channel ? "渠道已更新" : "渠道已创建");
      onDone();
    } catch (problem) {
      notify("bad", "保存渠道失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={channel ? `编辑渠道 · ${channel.name}` : "添加推送渠道"}
      sub="密钥类字段只保存环境变量名，不落明文"
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" disabled={!platform || testing} onClick={() => void test()}>
            {testing ? "检测中…" : "连通检测"}
          </button>
          <span className="fill" />
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button type="button" className="btn btn-primary" disabled={busy || !platform} onClick={() => void save()}>
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 14 }}>
        {!channel ? (
          <div className="platform-picker">
            {platforms.map((item) => (
              <button
                type="button"
                key={item.id}
                className={`platform-pick${platformId === item.id ? " is-active" : ""}`}
                onClick={() => changePlatform(item.id)}
              >
                <PlatformIcon icon={item.icon} label={item.name} size={28} />
                <span>{item.name}</span>
              </button>
            ))}
          </div>
        ) : null}
        {platform ? (
          <>
            <div className="alert info">{platform.note}</div>
            <Field label="渠道名称">
              <input className="input" value={name} onChange={(event) => setName(event.target.value)} placeholder={platform.name} />
            </Field>
            {platform.fields.map((field) => (
              <Field key={field.key} label={field.label} hint={field.secret ? "环境变量名，值由宿主环境提供" : undefined}>
                <input
                  className={`input${field.secret ? " mono" : ""}`}
                  value={settings[field.key] ?? ""}
                  onChange={(event) => setSettings((current) => ({ ...current, [field.key]: event.target.value }))}
                  placeholder={field.placeholder}
                />
              </Field>
            ))}
            <Field label="推送哪些事件" hint="只推送勾选的事件；分类开关仍以通知中心设置为准">
              <div className="row wrap" style={{ gap: 12 }}>
                {events.map((event) => (
                  <label key={event.id} className="row" style={{ gap: 5 }}>
                    <input
                      type="checkbox"
                      checked={subscribed.includes(event.id)}
                      onChange={() =>
                        setSubscribed((current) =>
                          current.includes(event.id) ? current.filter((item) => item !== event.id) : [...current, event.id],
                        )
                      }
                    />
                    <span className="small">{event.label}</span>
                  </label>
                ))}
              </div>
            </Field>
            <label className="switch">
              <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
              <span>启用这个渠道</span>
            </label>
          </>
        ) : (
          <Empty title="先选择平台" text="不同平台需要的设置不同，选好后填必要项即可。" />
        )}
        {result ? (
          <div className={`test-result ${result.ok ? "ok" : "bad"}`}>
            {result.ok ? "连通正常" : "连通失败"}：{result.detail}（{result.latencyMs}ms）
          </div>
        ) : null}
      </div>
    </Modal>
  );
}

/** 工作台通知提醒：轮询通知中心，把未读新条推给浏览器。 */
export function useDesktopNotifications() {
  useEffect(() => {
    if (typeof Notification === "undefined") return;
    let alive = true;
    const poll = async () => {
      try {
        const [settings, center] = await Promise.all([getNotificationSettings(), listNotificationCenter({ limit: 10 })]);
        if (!alive || !settings.desktop || typeof Notification === "undefined") return;
        const latest = center.items.filter((entry) => !entry.read);
        if (!latest.length) return;
        const previous = window.localStorage.getItem("modalprism.notifiedAt");
        const fresh = previous ? latest.filter((entry) => entry.createdAt > previous) : [];
        if (fresh.length && Notification.permission === "granted") {
          for (const entry of fresh.slice(0, 3)) {
            new Notification(entry.title, { body: entry.message });
          }
        }
        const next = latest[0]?.createdAt;
        if (next) window.localStorage.setItem("modalprism.notifiedAt", next);
      } catch {
        // 通知提醒是增强能力：失败不打断页面。
      }
    };
    void poll();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void poll();
    }, 30_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);
}
