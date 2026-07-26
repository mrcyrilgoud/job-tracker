import { NextResponse } from "next/server";

import {
  createCompany,
  listCompaniesWithWatches,
} from "@/lib/companies/service";

export const runtime = "nodejs";

export async function GET() {
  return NextResponse.json({ companies: listCompaniesWithWatches() });
}

export async function POST(request: Request) {
  try {
    const body = (await request.json()) as {
      name?: string;
      careersUrl?: string | null;
    };
    if (!body.name?.trim()) {
      return NextResponse.json({ error: "name is required" }, { status: 400 });
    }
    const company = createCompany({
      name: body.name,
      careersUrl: body.careersUrl,
    });
    return NextResponse.json({ company }, { status: 201 });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to create company" },
      { status: 400 },
    );
  }
}
