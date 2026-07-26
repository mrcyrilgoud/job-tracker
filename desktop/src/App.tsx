import { BrowserRouter, Route, Routes } from "react-router-dom";

import { Layout } from "@/components/Layout";
import { CompaniesPage } from "@/pages/CompaniesPage";
import { DocumentsPage } from "@/pages/DocumentsPage";
import { GmailPage } from "@/pages/GmailPage";
import { JobDetailPage } from "@/pages/JobDetailPage";
import { JobsPage } from "@/pages/JobsPage";
import { NewJobPage } from "@/pages/NewJobPage";

export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<JobsPage />} />
          <Route path="jobs/new" element={<NewJobPage />} />
          <Route path="jobs/:id" element={<JobDetailPage />} />
          <Route path="documents" element={<DocumentsPage />} />
          <Route path="companies" element={<CompaniesPage />} />
          <Route path="gmail" element={<GmailPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
