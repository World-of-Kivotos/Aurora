# Azure 应用注册指南（微软正版登录 client_id）

用途：为 Aurora 的微软正版登录注册 Azure/Entra 公共客户端应用，拿到 client_id。Aurora 用设备码流（device code flow），所以不需要回调地址、不需要 client secret。

步骤与结论均以 2026-07 的官方文档为准，关键处附来源。

## 零、开发期可先跳过注册

调试阶段可直接用官方启动器的调试 client_id `00000000402B5328`（祖父豁免、免 Mojang 审批、能跑通登录）。Aurora 的 client_id 走配置注入，发布前替换为自己注册并过审的即可。下面的注册与审批可以推迟到接近发布时办。

## 一、进注册后台

入口：<https://entra.microsoft.com>（Entra 管理中心，现官方主入口；portal.azure.com 搜 "App registrations" 等价）。用任意个人微软账号登录。

- 注册应用免费，不产生 Azure 费用。
- 走 entra.microsoft.com 这条入口不需要信用卡（从 azure.microsoft.com/free 建免费账户那条才会要卡做验证）。

如果登录时报 "Microsoft Services 租户" 错误，见第五节。

## 二、New registration 表单

1. Name：填 Aurora。注意不能含 Minecraft / Mojang / Microsoft / Xbox / Live / Discord / Hypixel（否则第四节审批被拒）。
2. Supported account types：选 "Personal accounts only"（个人账号）。Minecraft 只认消费者个人账号。
3. Redirect URI：留空（设备码流不用重定向）。
   - 仅当以后改用授权码流才需要：Authentication → Add a platform → "Mobile and desktop applications" → `http://localhost`。切勿选 "Web" 平台，否则被判为机密客户端、要求密钥，报 AADSTS7000218。

点 Register。

## 三、注册后必做配置

1. Allow public client flows（设备码流必开）：Authentication → Advanced settings → Allow public client flows → Yes → Save。不开会报 AADSTS7000218。
2. 不需要 client secret（公共客户端）。
3. 不需要在 API permissions 手动加 XboxLive.signin——门户列表里根本没有它，直接在登录请求的 scope 参数带 `XboxLive.signin offline_access` 即可。
4. 复制 client_id：Overview 页的 Application (client) ID（GUID，非密钥，可入 git）。顺手抄下 Directory (tenant) ID，第四节审批表要用。
5. 运行时端点必须用 consumers 租户（`login.microsoftonline.com/consumers/...`），不能用 common 或组织 tenant，否则 Xbox Live 步骤报错。

来源：<https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app> ；<https://minecraft.wiki/w/Microsoft_authentication> ；<https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration>

## 四、Mojang API 使用许可审批

打开 <https://aka.ms/mce-reviewappid>（匿名 Microsoft Form，个人账号可直接填、无需登录）。填：已读 EULA、联系邮箱（用注册 App 的同一邮箱）、请求类型 New AppID for Approval、Application Name（同样避开禁词）、Application ID（client_id）、Tenant ID、官网/仓库地址、Justification（说明 Aurora 是 MC 启动器为何要接入登录）。

- 大约按周批次审批，重复提交不会更快。
- 未过审：api.minecraftservices.com 返回 403，拿不到 Minecraft 令牌。
- 该审批制度上线前已存在的老应用被祖父豁免，无需补交。

来源：<https://help.minecraft.net/hc/en-us/articles/16254801392141>

## 五、故障排查：卡在 "Microsoft Services" 租户 / AADSTS16000

现象：个人微软账号（如 QQ 邮箱账号）登录 portal.azure.com / entra.microsoft.com 时，报 "所选用户帐户在租户 'Microsoft Services' 中不存在，且无法访问应用程序 c44b4083-3bb0-49c1-b47d-974e53cbdf3c"，或进了欢迎页但"没有订阅"、搜索不可用、弹 AADSTS16000 interaction_required。

成因：`c44b4083-...` 就是 Azure Portal 本身；`f8cdef31-a31e-4b4a-93e4-5f571e91255a`（显示名 "Microsoft Services"）是微软给"没有自己目录/订阅"的纯个人账号默认挂靠的公共受限租户。账号在这里只是受限访客，没有创建应用的权限，故门户取不到访问令牌。进一步地，微软自 2024 年起禁止"在目录外部创建应用程序"，纯个人账号已彻底不能注册应用，历史遗留的个人账号 app 也无法迁移，只能在新目录里重建。所以唯一出路是先给账号搞到一个属于自己的目录/租户。

重要：**"Entra 管理中心 → 管理租户 → 创建" 与深链接 `portal.azure.com/#create/Microsoft.AzureActiveDirectory` 对"零目录"账号走不通**——官方规定只有付费客户才能从 Entra 管理中心创建租户。别在这条路上耗。

解决（三选一，让账号拥有自己的目录）：
1. 先彻底登出：account.microsoft.com → 在所有位置退出；开无痕/隐私窗口，后续都在无痕窗口操作，避免 SSO 又把你带回受限租户。
2. 选一条获取目录的路径：
   - M365 开发人员计划（不绑卡，门槛：资格审核 + 约 90 天靠开发活动续期）：无痕窗口访问 <https://developer.microsoft.com/microsoft-365/dev-program>，用该账号 Join now，如实填开发用途，领取后得到一个全新 E5 开发者租户（分配 `xxx.onmicrosoft.com` 域名）。
   - M365 免费试用租户（不绑卡，验证手机号，30 天试用）：注册 Microsoft 365 商业版免费试用，自动生成新目录。门槛比开发者计划低（不做资格审核），但有试用期。
   - Azure 免费账户（要绑卡做身份验证、不扣费、目录永久、app 注册永久免费）：<https://azure.microsoft.com/free>，自动生成 Default Directory。最稳、最适合长期运行的正式产品，代价是绑一张卡。
3. 切换到新目录：entra.microsoft.com 右上角设置齿轮 → 切换目录 → 选新建的租户（别停在 Microsoft Services）。
4. 再进 App registrations → New registration，横幅消失即可正常注册。

选型提醒（针对 Aurora 这种要发布的产品）：开发者计划/试用租户都有有效期，生产环境一旦到期会导致所有用户登录中断，不适合正式发布用；正式发布应使用 Azure 免费账户这种永久目录。开发调试期根本不必自建，直接用第零节的调试 client_id 即可，把自建目录推迟到接近发布再办。应用注册在哪个租户不影响 Minecraft 登录——client_id 通用，登录仍走 consumers 端点。

来源：<https://learn.microsoft.com/en-us/azure/cost-management-billing/manage/microsoft-entra-id-free>（付费客户才能建租户 + Azure 免费账户需绑卡验证）；<https://learn.microsoft.com/en-us/office/developer-program/microsoft-365-developer-program-faq>（开发者计划）；<https://learn.microsoft.com/en-gb/answers/questions/2127352/how-to-move-an-azure-application-created-under-a-p>（个人账号 app 无法迁移）；应用 GUID 对照 <https://github.com/dmb2168/o365-appids/blob/master/ids.md>

## 六、已知怪癖

用自建 App 登录时，XSTS 阶段偶发 `2148916238`（误报未成年需加家庭组），同账号用官方 client_id 不报。这是微软侧老毛病，不是配置错误。
来源：<https://minecraft.wiki/w/Microsoft_authentication>
