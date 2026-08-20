// 外观设置的共享状态。
//
// 两个互不相邻的地方要用同一份数据：应用外壳渲染背景层，设置页改它。走 context 而不是各自
// 拉一次，是为了让设置页改完当场生效——否则得等下次进主页重新拉取，中间那段界面在说谎。

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import {
  applyGlassMode,
  EMPTY_APPEARANCE,
  loadAppearance,
  loadBuiltinBackgrounds,
} from "./appearance";
import type { AppearanceDto, BuiltinBackground } from "./ipc";

interface AppearanceState {
  appearance: AppearanceDto;
  /**
   * 内置背景登记表（一台游戏一张，随二进制发行）。没拉回来或拉失败时是空表，
   * 由 resolveBackground 兜住，界面退回纯纸底。
   */
  builtins: BuiltinBackground[];
  /** 设置页在每次改动后把后端返回的最新值推进来。 */
  applyAppearance: (next: AppearanceDto) => void;
}

const AppearanceContext = createContext<AppearanceState | null>(null);

export function AppearanceProvider({ children }: { children: ReactNode }) {
  const [appearance, setAppearance] = useState<AppearanceDto>(EMPTY_APPEARANCE);
  // 内置表进上下文而不是让外壳自己去拉：设置页的「恢复默认背景」按下去之后，
  // 呈现的正是这张表里的图，两处读的必须是同一份。表的内容编译进二进制，拉一次即定，
  // 所以只给读、不给写。
  const [builtins, setBuiltins] = useState<BuiltinBackground[]>([]);

  useEffect(() => {
    void loadAppearance().then(setAppearance);
    // 与外观各拉各的：内置表是常量，外观是配置，一方慢了不该拖住另一方先渲染。
    void loadBuiltinBackgrounds().then(setBuiltins);
  }, []);

  // 玻璃模式是 CSS 侧的全局开关（:root[data-glass]），所以它落在 DOM 根上而不是某棵子树里。
  // 跟着同一份 appearance 走：设置页改完把新 DTO 推进来，这里当场把属性改掉，
  // 不需要第二条通路，也就不存在「配置已改、界面还是旧材质」的中间态。
  useEffect(() => {
    applyGlassMode(appearance.glass, document.documentElement);
  }, [appearance.glass]);

  const applyAppearance = useCallback((next: AppearanceDto) => setAppearance(next), []);

  return (
    <AppearanceContext.Provider value={{ appearance, builtins, applyAppearance }}>
      {children}
    </AppearanceContext.Provider>
  );
}

export function useAppearance(): AppearanceState {
  const ctx = useContext(AppearanceContext);
  if (!ctx) throw new Error("useAppearance 必须在 AppearanceProvider 内使用");
  return ctx;
}
