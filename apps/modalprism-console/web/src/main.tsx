import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { ConsoleProvider } from "./lib/console";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/shell.css";
import "./styles/components.css";
import "./styles/pages.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("缺少挂载点");
}

createRoot(root).render(
  <StrictMode>
    <ConsoleProvider>
      <App />
    </ConsoleProvider>
  </StrictMode>,
);
