// 跨页面共用的配置写操作：把 YAML 声明文件的顶层 enabled 开关切换为目标状态。
import { configFile, configFiles, saveConfigFile } from "./api";
import { matchConfigFile, setYamlEnabled } from "./files";

export type ToggleResult = "enabled" | "disabled" | "unchanged";

/** notify 的最小签名，与 useConsole().notify 一致。 */
type Notify = (tone: "ok" | "bad" | "info", message: string, hint?: string) => void;

/**
 * 写回声明文件的 enabled 开关。
 * file 可以是模块目录里的绝对路径，会先匹配到配置目录的相对路径。
 */
export async function setModuleEnabled(file: string, enable: boolean): Promise<ToggleResult> {
  const files = await configFiles();
  const path = matchConfigFile(files, file);
  if (!path) {
    throw new Error(`${file} 不在配置目录里`);
  }
  if (!/\.ya?ml$/i.test(path)) {
    throw new Error("启用开关只能写在 YAML 声明文件里");
  }
  const body = await configFile(path);
  const next = setYamlEnabled(body.content, enable);
  if (next === body.content) {
    return "unchanged";
  }
  await saveConfigFile(path, next);
  return enable ? "enabled" : "disabled";
}

/** 切换模块启用状态并反馈结果；成功后刷新快照。供信源、通道、解析器页面共用。 */
export async function toggleModuleEnabled(
  item: { name: string; file: string; enabled: boolean },
  enable: boolean,
  kind: string,
  notify: Notify,
  refresh: () => void,
): Promise<void> {
  try {
    const result = await setModuleEnabled(item.file, enable);
    if (result === "unchanged") {
      notify(
        "info",
        enable ? `${kind}「${item.name}」本来就是启用状态` : `${kind}「${item.name}」本来就是停用状态`,
      );
      return;
    }
    notify(
      "ok",
      enable ? `已启用${kind}「${item.name}」` : `已停用${kind}「${item.name}」`,
      "已写入配置目录，采集进程约 2 秒内热加载。",
    );
    refresh();
  } catch (problem) {
    notify(
      "bad",
      enable ? `启用${kind}失败` : `停用${kind}失败`,
      problem instanceof Error ? problem.message : undefined,
    );
  }
}
