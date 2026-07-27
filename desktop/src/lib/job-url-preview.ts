import type { JobUrlPreview } from "@/lib/api";

export type JobPreviewFields = {
  title: string;
  companyName: string;
};

export function applyJobUrlPreview(
  current: JobPreviewFields,
  preview: JobUrlPreview,
): JobPreviewFields {
  return {
    title: current.title.trim() ? current.title : (preview.title ?? current.title),
    companyName: current.companyName.trim()
      ? current.companyName
      : (preview.companyName ?? current.companyName),
  };
}
