import fs from "node:fs";

import { NextResponse } from "next/server";

import { getDocumentFilePath } from "@/lib/documents/store";

export const runtime = "nodejs";

type Params = { params: Promise<{ id: string }> };

export async function GET(_request: Request, { params }: Params) {
  try {
    const { id } = await params;
    const { document, filePath } = getDocumentFilePath(id);
    const file = fs.readFileSync(filePath);
    return new NextResponse(file, {
      headers: {
        "Content-Type": document.mimeType,
        "Content-Disposition": `attachment; filename="${document.originalFilename.replace(/"/g, "")}"`,
      },
    });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Download failed" },
      { status: 404 },
    );
  }
}
