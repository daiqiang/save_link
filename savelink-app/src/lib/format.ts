// 格式化工具。

export function formatSize(bytes: number): string {
  if (bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return `${(bytes / 1024).toFixed(0)} KB`;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
}

function parseTimestamp(value: string): Date | null {
  const legacy = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})(?::(\d{2}))?$/.exec(value);
  const parsed = legacy
    ? new Date(
      Number(legacy[1]),
      Number(legacy[2]) - 1,
      Number(legacy[3]),
      Number(legacy[4]),
      Number(legacy[5]),
      Number(legacy[6] ?? 0),
    )
    : new Date(value);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

function pad2(value: number): string {
  return String(value).padStart(2, "0");
}

export function formatTimestamp(value: string | null | undefined): string {
  if (!value) return "—";
  const parsed = parseTimestamp(value);
  if (!parsed) return value;
  return `${parsed.getFullYear()}-${pad2(parsed.getMonth() + 1)}-${pad2(parsed.getDate())} ${pad2(parsed.getHours())}:${pad2(parsed.getMinutes())}`;
}

export function formatTimestampTime(value: string | null | undefined): string {
  if (!value) return "—";
  const parsed = parseTimestamp(value);
  if (!parsed) return value;
  return `${pad2(parsed.getHours())}:${pad2(parsed.getMinutes())}`;
}

export const REASON_LABEL: Record<string, string> = {
  manual: "手动创建",
  before_restore: "历史备份",
  auto: "自动快照",
};
