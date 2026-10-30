import { useCallback, useEffect, useMemo, useState } from "react";

import { createCollectionRun, getContent, listCollectionRuns, listContent, listPlans, listSources } from "../lib/api";
import { isHtmlBody } from "../lib/article";
import { contentKindLabel, relative, stamp, statusTone } from "../lib/format";
import { navigate } from "../lib/router";
import type { CollectionRun, ContentDetail, ContentItem, Plan, Source } from "../lib/types";
import { ArticleBody } from "./ArticleBody";
import { IconHistory, IconRefresh } from "./icons";
import { Card, Drawer, Empty, Loading, Problem, useUi } from "./ui";

const RUN_LABELS: Record<string, string> = {
  succeeded: "成功",
  failed: "失败",
  running: "执行中",
  queued: "排队",
  cancelled: "已取消",
};

/**
 * 采集记录抽屉：一次采集 = 一个任务，上游是信源（和可选的计划），下游是它产出的内容。
 * 这里把这条链路放在同一屏内：左侧选一条记录，右侧看这次产出的内容，内容可再展开速览，
 * 由用户看完再决定「再采集一次 / 去信源 / 去计划 / 打开完整内容」，不用先跳页再回来。
 */
export function RunsDrawer({
  title,
  sourceId,
  planId,
  initialRunId,
  onClose,
  projectId,
}: {
  title: string;
  sourceId?: string;
  planId?: string;
  /** 从某条记录直接打开时选中它。 */
  initialRunId?: string;
  onClose: () => void;
  /** 跨项目只读预览用；缺省跟随当前项目。 */
  projectId?: string;
}) {
  const { notify, run } = useUi();
  const [runs, setRuns] = useState<CollectionRun[] | null>(null);
  const [sources, setSources] = useState<Source[]>([]);
  const [plans, setPlans] = useState<Plan[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [active, setActive] = useState<string | null>(null);
  const [produced, setProduced] = useState<Record<string, ContentItem[]>>({});
  const [loadingItems, setLoadingItems] = useState(false);
  const [peek, setPeek] = useState<ContentDetail | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [runPage, sourcePage, planPage] = await Promise.all([
        listCollectionRuns({ sourceId, planId, limit: 30, projectId }),
        listSources({ limit: 200, projectId }),
        listPlans({ limit: 200, projectId }),
      ]);
      setRuns(runPage.items);
      setSources(sourcePage.items);
      setPlans(planPage.items);
      setActive((current) => current ?? initialRunId ?? runPage.items[0]?.id ?? null);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [sourceId, planId, projectId, initialRunId]);

  useEffect(() => {
    void load();
  }, [load]);

  const sourceName = useCallback(
    (id: string) => sources.find((item) => item.id === id)?.name ?? id,
    [sources],
  );
  const planName = useCallback(
    (id?: string | null) => (id ? plans.find((item) => item.id === id)?.name ?? id : ""),
    [plans],
  );

  const runsOfSource = useMemo(() => {
    const map = new Map<string, CollectionRun[]>();
    for (const run of runs ?? []) {
      const list = map.get(run.sourceId) ?? [];
      list.push(run);
      map.set(run.sourceId, list);
    }
    return map;
  }, [runs]);

  const activeRun = useMemo(() => (runs ?? []).find((item) => item.id === active) ?? null, [runs, active]);

  /** 一次记录的时间窗：从它开始，到同一信源的下一条记录为止；没有下一条就到现在。 */
  const windowOf = useCallback(
    (run: CollectionRun) => {
      const same = runsOfSource.get(run.sourceId) ?? [];
      const index = same.findIndex((item) => item.id === run.id);
      const newer = index > 0 ? same[index - 1] : undefined;
      return { from: run.createdAt, to: newer?.createdAt ?? "" };
    },
    [runsOfSource],
  );

  // 选中某条记录时按时间窗取该信源的内容：内容没有运行号，只能靠「信源 + 入库时间」归组。
  useEffect(() => {
    if (!activeRun || produced[activeRun.id]) return;
    const { from, to } = windowOf(activeRun);
    let alive = true;
    setLoadingItems(true);
    void (async () => {
      try {
        const page = await listContent({ sourceId: activeRun.sourceId, limit: 100, status: "active", projectId });
        const rows = page.items.filter((item) => item.createdAt >= from && (!to || item.createdAt < to)).slice(0, 12);
        if (alive) setProduced((current) => ({ ...current, [activeRun.id]: rows }));
      } catch {
        if (alive) setProduced((current) => ({ ...current, [activeRun.id]: [] }));
      } finally {
        if (alive) setLoadingItems(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [activeRun, produced, windowOf, projectId]);

  async function collectAgain() {
    if (!activeRun) return;
    setBusy(true);
    try {
      const operation = await createCollectionRun({ sourceId: activeRun.sourceId, mode: "normal", reason: "manual" });
      await run(operation, `手动采集 ${sourceName(activeRun.sourceId)}`);
      setProduced({});
      await load();
      notify("ok", "已发起采集", "完成后可刷新查看本次产出。");
    } catch (problem) {
      notify("bad", "采集失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function peekContent(id: string) {
    try {
      const meta = await getContent(id);
      setPeek(meta.data);
    } catch (problem) {
      notify("bad", "读取内容失败", problem instanceof Error ? problem.message : undefined);
    }
  }

  const counts = useMemo(() => {
    const total = (runs ?? []).length;
    const failed = (runs ?? []).filter((item) => item.status === "failed").length;
    const running = (runs ?? []).filter((item) => item.status === "running" || item.status === "queued").length;
    return { total, failed, running };
  }, [runs]);

  return (
    <Drawer
      title={title}
      sub={loading && !runs ? "读取采集记录…" : `最近 ${counts.total} 次 · ${counts.running} 个在途 · ${counts.failed} 个失败`}
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn" onClick={onClose}>关闭</button>
          <button type="button" className="btn btn-primary" onClick={() => void collectAgain()} disabled={busy || !activeRun}>
            {busy ? "提交中…" : "再采集一次"}
          </button>
        </>
      }
    >
      {error ? <Problem error={error} onRetry={() => void load()} /> : null}
      {loading && !runs ? <Loading lines={6} /> : null}
      {runs && runs.length === 0 ? (
        <Empty
          title="暂无采集记录"
          text="计划触发与手动采集的记录；首次采集完成后可查看产出。"
          action={<button type="button" className="btn btn-sm" onClick={() => void collectAgain()} disabled={!activeRun}>立即采集一次</button>}
        />
      ) : null}
      {runs && runs.length > 0 ? (
        <div className="split-2 runs-layout">
          <Card title="采集记录" sub="按时间倒序；选择一行查看本次产出" flush>
            <div className="list">
              {(runs ?? []).map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className={`list-row clickable${item.id === active ? " is-active" : ""}`}
                  onClick={() => {
                    setActive(item.id);
                    setPeek(null);
                  }}
                >
                  <span className={`dot ${item.status === "failed" ? "bad" : item.status === "succeeded" ? "ok" : "warn"}`} />
                  <span className="list-main">
                    <span className="list-title">
                      {sourceName(item.sourceId)}
                      <span className={`badge ${statusTone(item.status)}`} style={{ marginLeft: 6 }}>{RUN_LABELS[item.status] ?? item.status}</span>
                      <span className="badge neutral" style={{ marginLeft: 4 }}>{item.mode === "manual" ? "手动" : "计划"}</span>
                    </span>
                    <span className="list-meta">
                      <span>{relative(item.createdAt)}</span>
                      <span>{stamp(item.createdAt)}</span>
                      {item.planId ? <span>计划 {planName(item.planId)}</span> : null}
                      {item.attempt > 1 ? <span>第 {item.attempt} 次尝试</span> : null}
                      <span className="mono">{item.queue}</span>
                    </span>
                    {item.error ? <span className="list-meta" style={{ color: "var(--danger)" }}>{item.error}</span> : null}
                  </span>
                </button>
              ))}
            </div>
          </Card>

          <div className="stack" style={{ gap: 14 }}>
            <Card
              title="本次产出"
              sub={activeRun ? `${sourceName(activeRun.sourceId)} · ${stamp(activeRun.createdAt)} · 手动/计划派发` : undefined}
              flush
              action={
                activeRun ? (
                  <div className="row" style={{ gap: 6 }}>
                    <button type="button" className="btn btn-sm" onClick={() => navigate(`sources?query=${encodeURIComponent(activeRun.sourceId)}`)}>
                      信源
                    </button>
                    {activeRun.planId ? (
                      <button type="button" className="btn btn-sm" onClick={() => navigate(`plans?query=${encodeURIComponent(activeRun.planId ?? "")}`)}>
                        计划
                      </button>
                    ) : null}
                  </div>
                ) : null
              }
            >
              {!activeRun ? (
                <Empty title="先选一条采集记录" text="左侧选择一行，查看该次运行的上游与产出。" />
              ) : loadingItems ? (
                <div className="card-body"><Loading lines={3} /></div>
              ) : (produced[activeRun.id] ?? []).length === 0 ? (
                <Empty
                  title="这次没有新增内容"
                  text={activeRun.status === "failed" ? "本次运行失败，无产出；查看错误后可重新采集。" : "无新增条目，或新增条目早于最近 100 条内容。重复采集的更新不计入新增。"}
                />
              ) : (
                <div className="list">
                  {(produced[activeRun.id] ?? []).map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`list-row clickable${peek?.id === item.id ? " is-active" : ""}`}
                      onClick={() => void peekContent(item.id)}
                    >
                      <span className="list-main">
                        <span className="list-title">{item.title}</span>
                        <span className="list-meta">
                          <span>{contentKindLabel(item.kind)}</span>
                          <span>{item.publishedAt ? `发布 ${stamp(item.publishedAt)}` : "未提供发布时间"}</span>
                          <span>入库 {relative(item.createdAt)}</span>
                        </span>
                      </span>
                      <IconHistory size={14} />
                    </button>
                  ))}
                </div>
              )}
              <div className="card-body small muted" style={{ paddingTop: 8 }}>
                统计范围：本次运行开始至下一次运行前入库的内容；重复采集的更新不计入。
              </div>
            </Card>

            {peek ? (
              <Card
                title="内容速览"
                sub={`${peek.sourceName} · 入库 ${stamp(peek.createdAt)}`}
                action={
                  <div className="row" style={{ gap: 6 }}>
                    <button type="button" className="btn btn-sm" onClick={() => navigate(`content?id=${encodeURIComponent(peek.id)}`)}>
                      打开完整内容
                    </button>
                    <button type="button" className="btn btn-sm" onClick={() => setPeek(null)}>收起</button>
                  </div>
                }
              >
                <div className="stack" style={{ gap: 10 }}>
                  <strong>{peek.title}</strong>
                  {peek.tags.length ? (
                    <div className="row wrap" style={{ gap: 6 }}>
                      {peek.tags.map((tag) => <span className="badge neutral" key={tag}>{tag}</span>)}
                    </div>
                  ) : null}
                  {peek.body?.text ? (
                    isHtmlBody(peek.body.kind, peek.body.mediaType, peek.body.text) ? (
                      <ArticleBody html={peek.body.text.slice(0, 4_000)} base={contentUrl(peek)} bodyKind={peek.body.kind} mediaType={peek.body.mediaType} />
                    ) : (
                      <div className="body-text">
                        {peek.body.text.slice(0, 1_200).split(/\n{2,}/).map((paragraph, index) => <p key={index}>{paragraph}</p>)}
                      </div>
                    )
                  ) : (
                    <span className="small muted">该内容仅含结构化字段，完整信息见内容详情。</span>
                  )}
                </div>
              </Card>
            ) : null}
          </div>
        </div>
      ) : null}
    </Drawer>
  );
}

/** 原文地址：优先取结构面的 locator.url，其次顶层 url。 */
function contentUrl(detail: ContentDetail): string {
  const struct = (detail.struct ?? {}) as Record<string, unknown>;
  const locator = struct.locator;
  const located = locator && typeof locator === "object" ? (locator as Record<string, unknown>).url : undefined;
  const value = struct.url ?? located;
  return typeof value === "string" ? value : "";
}
