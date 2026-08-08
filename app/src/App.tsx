// 路由与外壳装配。用 HashRouter：Tauri 生产环境从静态文件加载，hash 路由不依赖服务端处理深链接。
// 加新页 = 在 AppShell 子路由下加一条 <Route>，并在 Sidebar 的 TOP/BOTTOM 里加对应导航项。

import { HashRouter, Routes, Route, Navigate } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { Home } from "./pages/Home";
import { Account } from "./pages/Account";
import { Versions } from "./pages/Versions";
import { InstanceDetail } from "./pages/InstanceDetail";
import { Download } from "./pages/Download";
import { Settings } from "./pages/Settings";

export default function App() {
  return (
    <HashRouter>
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
    </HashRouter>
  );
}
