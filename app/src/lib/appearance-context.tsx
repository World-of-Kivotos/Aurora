// 外观设置的共享状态。
//
// 两个互不相邻的地方要用同一份数据：应用外壳渲染背景层，设置页改它。走 context 而不是各自
// 拉一次，是为了让设置页改完当场生效——否则得等下次进主页重新拉取，中间那段界面在说谎。

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { EMPTY_APPEARANCE, loadAppearance } from "./appearance";
import type { AppearanceDto } from "./ipc";

interface AppearanceState {
  appearance: AppearanceDto;
  /** 设置页在每次改动后把后端返回的最新值推进来。 */
  applyAppearance: (next: AppearanceDto) => void;
}

const AppearanceContext = createContext<AppearanceState | null>(null);

export function AppearanceProvider({ children }: { children: ReactNode }) {
  const [appearance, setAppearance] = useState<AppearanceDto>(EMPTY_APPEARANCE);

  useEffect(() => {
    void loadAppearance().then(setAppearance);
  }, []);

  const applyAppearance = useCallback((next: AppearanceDto) => setAppearance(next), []);

  return (
    <AppearanceContext.Provider value={{ appearance, applyAppearance }}>
      {children}
    </AppearanceContext.Provider>
  );
}

export function useAppearance(): AppearanceState {
  const ctx = useContext(AppearanceContext);
  if (!ctx) throw new Error("useAppearance 必须在 AppearanceProvider 内使用");
  return ctx;
}
