import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { listProjectResources, listProjects } from "./api";
import type { Project, ProjectResource } from "./types";

/**
 * 全局当前项目上下文：
 * - currentProjectId 持久化在 localStorage，刷新后保持；
 * - 切到不存在的项目时回落到默认项目；
 * - 暴露项目清单与当前项目的资源绑定，供各页过滤/标注。
 */
interface ProjectContextValue {
  projects: Project[];
  currentProject: Project | null;
  currentProjectId: string | null;
  setCurrentProjectId: (id: string) => void;
  boundIds: (kind: string) => string[];
  reload: () => Promise<void>;
  loading: boolean;
}

const STORAGE_KEY = "modalprism.currentProjectId";
const ProjectContext = createContext<ProjectContextValue | null>(null);

export function ProjectProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [currentProjectId, setCurrentId] = useState<string | null>(() =>
    window.localStorage.getItem(STORAGE_KEY),
  );
  const [bindings, setBindings] = useState<ProjectResource[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const page = await listProjects();
      setProjects(page.items);
      setCurrentId((current) => {
        if (current && page.items.some((item) => item.id === current)) return current;
        const fallback = page.items.find((item) => item.isDefault)?.id ?? page.items[0]?.id ?? null;
        return fallback;
      });
    } catch {
      // collector 不可达时保留外壳，项目清单留空。
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!currentProjectId) {
      setBindings([]);
      return;
    }
    window.localStorage.setItem(STORAGE_KEY, currentProjectId);
    let alive = true;
    listProjectResources(currentProjectId)
      .then((page) => {
        if (alive) setBindings(page.items);
      })
      .catch(() => {
        if (alive) setBindings([]);
      });
    return () => {
      alive = false;
    };
  }, [currentProjectId]);

  const setCurrentProjectId = useCallback((id: string) => setCurrentId(id), []);

  const value = useMemo<ProjectContextValue>(() => {
    const bound = (kind: string) =>
      bindings.filter((item) => item.kind === kind).map((item) => item.id);
    return {
      projects,
      currentProject: projects.find((item) => item.id === currentProjectId) ?? null,
      currentProjectId,
      setCurrentProjectId,
      boundIds: bound,
      reload: load,
      loading,
    };
  }, [projects, currentProjectId, setCurrentProjectId, bindings, load, loading]);

  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>;
}

export function useCurrentProject(): ProjectContextValue {
  const value = useContext(ProjectContext);
  if (!value) throw new Error("useCurrentProject 必须在 ProjectProvider 内使用");
  return value;
}

/** 页头项目标注：显示当前项目名，未选择时不渲染。 */
export function ProjectScopeBadge() {
  const { currentProject } = useCurrentProject();
  if (!currentProject) return null;
  return (
    <span className="badge neutral" title="全局当前项目">
      项目：{currentProject.name}
    </span>
  );
}
