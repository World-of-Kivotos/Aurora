// 占位页：本轮只做导航入口，页面正文用统一空态说明“后续接入”。
// 页面 agent 接手某页时，把这里替换为真实内容即可（复用同一 PageHeader + Card + EmptyState 骨架）。

import type { ReactNode } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "./PageHeader";
import { Card } from "./Card";
import { EmptyState } from "./EmptyState";
import { pageItem } from "../lib/motion";

interface PlaceholderPageProps {
  title: string;
  subtitle: string;
  icon: ReactNode;
  note: string;
}

export function PlaceholderPage({ title, subtitle, icon, note }: PlaceholderPageProps) {
  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader title={title} subtitle={subtitle} />
      </motion.div>
      <Card variants={pageItem}>
        <EmptyState icon={icon} title={note} />
      </Card>
    </>
  );
}
