import { describe, expect, it } from "vitest";
import { formatLocalTime } from "./format";
import { loadTheme, saveTheme, applyTheme } from "./theme";

describe("theme", () => {
  it("defaults to light when unset", () => {
    localStorage.clear();
    expect(loadTheme()).toBe("light");
  });

  it("persists selection", () => {
    saveTheme("dark");
    expect(loadTheme()).toBe("dark");
    applyTheme("dark");
    expect(document.documentElement.classList.contains("theme-dark")).toBe(true);
  });
});

describe("formatLocalTime", () => {
  it("formats iso date", () => {
    const s = formatLocalTime("2026-01-15T12:00:00.000Z");
    expect(s).not.toBe("—");
  });
});
