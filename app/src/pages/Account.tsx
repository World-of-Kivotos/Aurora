import { PlaceholderPage } from "../components/PlaceholderPage";
import { UserIcon } from "../components/icons";

export function Account() {
  return (
    <PlaceholderPage
      title="账户"
      subtitle="管理微软正版与离线账户"
      icon={<UserIcon />}
      note="账户管理将在后续版本接入"
    />
  );
}
