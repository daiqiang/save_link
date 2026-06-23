// 格式化工具。

export function formatSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return `${(bytes / 1024).toFixed(0)} KB`;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
}

export const REASON_LABEL: Record<string, string> = {
  manual: "手动创建",
  before_restore: "恢复前自动备份",
  auto: "自动快照",
};
