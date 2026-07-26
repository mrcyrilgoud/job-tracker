export function classifyEmail(input: {
  subject: string;
  snippet: string;
  fromAddress: string;
  companyName: string;
  jobTitle: string;
}) {
  const haystack = `${input.subject} ${input.snippet} ${input.fromAddress}`.toLowerCase();
  const company = input.companyName.toLowerCase();
  const title = input.jobTitle.toLowerCase();
  const hasCompany = company.length > 2 && haystack.includes(company);
  const hasTitle = title.length > 2 && haystack.includes(title);
  const applicationSignals = [
    "application",
    "interview",
    "thank you for applying",
    "next steps",
    "offer",
    "unfortunately",
    "status update",
  ];
  const hasSignal = applicationSignals.some((signal) => haystack.includes(signal));

  if (hasCompany && hasTitle && hasSignal) {
    return "high" as const;
  }
  if ((hasCompany || hasTitle) && hasSignal) {
    return "medium" as const;
  }
  if (hasCompany || hasTitle) {
    return "low" as const;
  }
  return null;
}
