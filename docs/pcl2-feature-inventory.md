# PCL2 功能盘点摘要

盘点日期：2026-07-13。方法：六路并行扫描 Meloong-Git/PCL 源码与官方文档/社区资料，完整性审计补扫 31 条，合并去重后 232 条唯一功能，全量数据见 [pcl2-feature-matrix.json](pcl2-feature-matrix.json)（字段：name/detail/layer/module/priority/source）。

## 统计

- 优先级：mvp 57 / v1 85 / later 90
- 分层：backend 118 / both 77 / ui 37
- 模块分布：none(纯UI) 35、config 30、modplatform 30、launch 29、download 26、modloader 15、auth 13、version 12、java 12、crash 9、instance 9、link 7、skin 5

## 架构级要点（合并代理结论）

1. MVP 红线是四条主链路：版本识别与继承合并、Java 探测与自动下载、多线程下载引擎 + BMCLAPI 镜像切换（国内网络刚需）、启动参数拼装到进程启动。外加登录状态机、崩溃基础检测、双平台 Mod 搜索、Mod 增删启禁。
2. 三类功能依赖第三方/自建服务，暂缓并需产品决策：联机大厅（EasyTier 节点网络，7 条）、统一通行证 Nide8（商业验证服务）、MCIM Mod 镜像。
3. PCL 开源仓库中本就是空实现的部分（自动更新下载替换、百宝箱、公告网络逻辑、导入导出设置）只能当功能清单，不能当实现参考。
4. 约 40 处 UI 视觉/彩蛋功能深度绑定 WPF/VB.NET，迁移到 iced 只能迁移"功能意图"，按重新设计估算工时，全部 later。
5. PCL 自身历史版本迁移逻辑（4 处）对新项目无意义，不照搬。
6. Quilt 加载器自动安装是 PCL2 明确未实现的空白，Aurora 已列入 v1 范围作为差异化点。

## 对应关系

功能矩阵的 module 字段与 aurora workspace 的 crate 对应关系见 [architecture.md](architecture.md) 第四节；modloader 模块归入 aurora-install，crash 模块本轮并入 aurora-launch 的基础检测。
