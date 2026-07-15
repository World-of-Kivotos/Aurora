import { PlaceholderPage } from "../components/PlaceholderPage";
import { LayersIcon } from "../components/icons";

export function Versions() {
  return (
    <PlaceholderPage
      title="版本"
      subtitle="安装与管理游戏版本"
      icon={<LayersIcon />}
      note="版本下载将在后续版本接入"
    />
  );
}
