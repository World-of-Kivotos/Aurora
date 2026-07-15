// 路由与外壳装配。用 HashRouter：Tauri 生产环境从静态文件加载，hash 路由不依赖服务端处理深链接。
// 加新页 = 在 AppShell 子路由下加一条 <Route>，并在 Sidebar 的 TOP/BOTTOM 里加对应导航项。

import { HashRouter, Routes, Route, Navigate } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { Home } from "./pages/Home";
import { Account } from "./pages/Account";
import { Versions } from "./pages/Versions";
import { Resources } from "./pages/Resources";
import { Settings } from "./pages/Settings";

export default function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<AppShell />}>
          <Route index element={<Home />} />
          <Route path="account" element={<Account />} />
          <Route path="versions" element={<Versions />} />
          <Route path="resources" element={<Resources />} />
          <Route path="settings" element={<Settings />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </HashRouter>
  );
}
