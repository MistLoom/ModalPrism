import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { ApiError, commits, ping, snapshot } from "./api";
import type { Snapshot } from "./api";
import type { Commit, Connection, Source } from "./types";

export type LoadState = "loading" | "ready" | "offline" | "error";

interface ConsoleState {
  state: LoadState;
  error: string | null;
  connection: Connection | null;
  snapshot: Snapshot | null;
  sources: Source[];
  timeline: Commit[];
  liveCursor: number | null;
  cursor: number | null;
  setCursor: (value: number | null) => void;
  refresh: () => void;
  reload: () => void;
  busy: boolean;
  toasts: Toast[];
  notify: (tone: ToastTone, message: string, hint?: string) => void;
  dismiss: (id: number) => void;
  palette: boolean;
  setPalette: (open: boolean) => void;
  now: number;
}

export type ToastTone = "ok" | "warn" | "bad" | "info";

export interface Toast {
  id: number;
  tone: ToastTone;
  message: string;
  hint?: string;
}

const Context = createContext<ConsoleState | null>(null);

export function ConsoleProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<LoadState>("loading");
  const [error, setError] = useState<string | null>(null);
  const [connection, setConnection] = useState<Connection | null>(null);
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [timeline, setTimeline] = useState<Commit[]>([]);
  const [cursor, setCursor] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [palette, setPalette] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const seq = useRef(0);
  const toastSeq = useRef(0);

  const notify = useCallback((tone: ToastTone, message: string, hint?: string) => {
    toastSeq.current += 1;
    const id = toastSeq.current;
    setToasts((list) => [...list, { id, tone, message, hint }].slice(-4));
    window.setTimeout(() => {
      setToasts((list) => list.filter((item) => item.id !== id));
    }, tone === "bad" ? 9000 : 5200);
  }, []);

  const dismiss = useCallback((id: number) => {
    setToasts((list) => list.filter((item) => item.id !== id));
  }, []);

  const load = useCallback(async () => {
    seq.current += 1;
    const mine = seq.current;
    setBusy(true);
    try {
      const health = await ping();
      if (mine !== seq.current) {
        return;
      }
      setConnection(health);
      if (!health.online) {
        setState("offline");
        setError(health.message ?? null);
        setSnap(null);
        setBusy(false);
        return;
      }
      const [data, history] = await Promise.all([snapshot(), commits(0, 24).catch(() => [])]);
      if (mine !== seq.current) {
        return;
      }
      setSnap(data);
      const ordered = [...history].sort((left, right) => left.snapshot - right.snapshot);
      setTimeline(ordered);
      setState("ready");
      setError(null);
    } catch (problem) {
      if (mine !== seq.current) {
        return;
      }
      setState(problem instanceof ApiError && problem.status === 0 ? "offline" : "error");
      setError(problem instanceof Error ? problem.message : "读取失败");
    } finally {
      if (mine === seq.current) {
        setBusy(false);
      }
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 心跳慢一点：控制台是观察者，不做高频轮询。
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    const interval = window.setInterval(() => {
      if (document.visibilityState === "visible") {
        void load();
      }
    }, 30_000);
    return () => window.clearInterval(interval);
  }, [load]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPalette((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const value = useMemo<ConsoleState>(
    () => ({
      state,
      error,
      connection,
      snapshot: snap,
      sources: snap?.sources ?? [],
      timeline,
      liveCursor: timeline.length ? timeline[timeline.length - 1].snapshot : null,
      cursor,
      setCursor,
      refresh: () => {
        void load();
      },
      reload: () => {
        void load();
      },
      busy,
      toasts,
      notify,
      dismiss,
      palette,
      setPalette,
      now,
    }),
    [state, error, connection, snap, timeline, cursor, busy, toasts, notify, dismiss, palette, now, load],
  );

  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useConsole(): ConsoleState {
  const value = useContext(Context);
  if (!value) {
    throw new Error("ConsoleProvider 未挂载");
  }
  return value;
}
