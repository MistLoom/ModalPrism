import { describe, expect, it } from "vitest";
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
