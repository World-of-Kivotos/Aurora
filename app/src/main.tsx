import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { MotionPrefProvider } from "./lib/motion-pref";
import { ToastProvider } from "./components/Toast";
import "./styles/app.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <MotionPrefProvider>
      <ToastProvider>
        <App />
      </ToastProvider>
    </MotionPrefProvider>
  </React.StrictMode>,
);
