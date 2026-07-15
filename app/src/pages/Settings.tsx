import { PlaceholderPage } from "../components/PlaceholderPage";
import { SettingsIcon } from "../components/icons";

export function Settings() {
  return (
    <PlaceholderPage
      title="设置"
      subtitle="下载源、内存与目录"
      icon={<SettingsIcon />}
      note="设置项将在后续版本接入"
    />
  );
}
