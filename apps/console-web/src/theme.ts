const KEY = "modalprism-console-theme";

export type ThemeMode = "light" | "dark" | "system";

export function loadTheme(): ThemeMode {
  const v = localStorage.getItem(KEY);
  if (v === "light" || v === "dark" || v === "system") return v;
  return "light";
}

export function saveTheme(mode: ThemeMode) {
  localStorage.setItem(KEY, mode);
  applyTheme(mode);
}

export function applyTheme(mode: ThemeMode) {
  const root = document.documentElement;
  root.dataset.theme = mode;
  const dark =
    mode === "dark" ||
    (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  root.classList.toggle("theme-dark", dark);
  root.classList.toggle("theme-light", !dark);
}
