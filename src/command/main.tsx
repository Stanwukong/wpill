import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { CommandApp } from "./CommandApp";
import "../styles/global.css";
import "../features/agent/agent.css";
import "./command.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <CommandApp />
  </StrictMode>,
);
