import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { MotionPrefProvider } from "./lib/motion-pref";
import "./styles/app.css";

// ToastProvider 不在这里：提示浮层要按「当前是否压在背景图上」换材质，
// 得同时读到路由与外观，因此下沉到 App 里的 Router 与 AppearanceProvider 之内。
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <MotionPrefProvider>
      <App />
    </MotionPrefProvider>
  </React.StrictMode>,
);
