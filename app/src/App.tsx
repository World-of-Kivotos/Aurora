// 路由与外壳装配。用 HashRouter：Tauri 生产环境从静态文件加载，hash 路由不依赖服务端处理深链接。
// 加新页 = 在 AppShell 子路由下加一条 <Route>，并在 Sidebar 的 TOP/BOTTOM 里加对应导航项。
//
// 首次启动先过初次设定：配置还没落过盘时整个路由都不挂载，避免主页先扫一遍空目录、
// 用户刚设完目录又要手动刷新。设定完成把 phase 推到 ready，路由这才装配。

import { useCallback, useEffect, useState } from "react";
import { HashRouter, Routes, Route, Navigate } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { FirstRunWizard } from "./components/FirstRunWizard";
import { Home } from "./pages/Home";
import { Account } from "./pages/Account";
import { Versions } from "./pages/Versions";
import { InstanceDetail } from "./pages/InstanceDetail";
import { Download } from "./pages/Download";
import { Settings } from "./pages/Settings";
import { AppearanceProvider } from "./lib/appearance-context";
import { ToastProvider } from "./components/Toast";
import { isFirstRun } from "./lib/ipc";

/** 启动闸门：探测中什么都不画，免得白屏一闪之后又跳向导。 */
type Phase = "probing" | "first-run" | "ready";

export default function App() {
  const [phase, setPhase] = useState<Phase>("probing");

  const probe = useCallback(async () => {
    try {
      setPhase((await isFirstRun()) ? "first-run" : "ready");
    } catch {
      // 探测失败不该把人挡在门外：直接进主界面，配置真有问题会在各页面以错误块呈现。
      setPhase("ready");
    }
  }, []);

  useEffect(() => {
    void probe();
  }, [probe]);

  if (phase === "probing") return null;
  if (phase === "first-run") return <FirstRunWizard onDone={() => setPhase("ready")} />;

  // 外观 Provider 包在路由外：外壳要拿它渲染背景，设置页要改它，两处共用同一份状态，
  // 设置页改完当场生效而不必等下次进主页重新拉。
  return (
    <AppearanceProvider>
      <HashRouter>
        {/* Toast 挂在 Router 与 AppearanceProvider 之内：它要按「当前是否压在背景图上」
            换材质，两样都得读得到。所有 useToast 调用点都在下面的路由子树里，
            初次设定向导不用 toast，因此下沉不影响任何调用方。 */}
        <ToastProvider>
          <Routes>
            <Route element={<AppShell />}>
              <Route index element={<Home />} />
              <Route path="account" element={<Account />} />
              <Route path="versions" element={<Versions />} />
              {/* 实例卷宗：id 即版本目录名，可能含空格与中文，路由参数天然承载不需要额外编码。 */}
              <Route path="versions/:id" element={<InstanceDetail />} />
              <Route path="download" element={<Download />} />
              <Route path="settings" element={<Settings />} />
              <Route path="*" element={<Navigate to="/" replace />} />
            </Route>
          </Routes>
        </ToastProvider>
      </HashRouter>
    </AppearanceProvider>
  );
}
