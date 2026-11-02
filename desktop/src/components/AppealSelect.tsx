import { ScoreSelect, type ScoreOption, type ScoreSelectWrapperProps } from "./ScoreSelect";

export const APPEAL_SCALE_LABEL = "Appeal (1–5, 5=best)";
const OPTIONS: readonly ScoreOption[] = [
  { value: "", label: "Unscored", compact: "—" },
  { value: "1", label: "1 — least appealing", compact: "1" },
  { value: "2", label: "2", compact: "2" },
  { value: "3", label: "3", compact: "3" },
  { value: "4", label: "4", compact: "4" },
  { value: "5", label: "5 — most appealing", compact: "5" },
];
export function AppealSelect(props: ScoreSelectWrapperProps) {
  return <ScoreSelect {...props} scaleLabel={APPEAL_SCALE_LABEL} options={OPTIONS} />;
}
