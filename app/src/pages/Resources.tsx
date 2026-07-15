import { PlaceholderPage } from "../components/PlaceholderPage";
import { PackageIcon } from "../components/icons";

export function Resources() {
  return (
    <PlaceholderPage
      title="资源"
      subtitle="搜索并安装 Mod、资源包等"
      icon={<PackageIcon />}
      note="资源下载将在后续版本接入"
    />
  );
}
