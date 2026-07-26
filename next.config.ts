import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  serverExternalPackages: ["better-sqlite3", "keytar"],
  turbopack: {
    root: process.cwd(),
  },
};

export default nextConfig;
