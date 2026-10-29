import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { applySavedAccent, applySavedMode, followSystemMode } from "./lib/theme";
import "./styles.css";

applySavedAccent();
applySavedMode();
// Unless Settings picks Light or Dark, follow the system as it changes.
followSystemMode(() => undefined);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
