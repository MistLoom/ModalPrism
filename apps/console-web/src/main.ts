import { createApp } from "./app";
import { applyTheme, loadTheme } from "./theme";
import "./styles.css";

const themeQuery = new URLSearchParams(location.search).get("theme");
if (themeQuery === "light" || themeQuery === "dark") {
  applyTheme(themeQuery);
} else {
  applyTheme(loadTheme());
}
createApp(document.querySelector("#app") as HTMLElement);
