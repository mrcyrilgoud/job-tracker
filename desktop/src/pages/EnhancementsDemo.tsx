import { useState } from "react";
import { Link } from "react-router-dom";

import { StatusQuickChange } from "@/components/StatusQuickChange";
import { KeyboardShortcutsHelp } from "@/components/KeyboardShortcutsHelp";
import { JobsBoardViewEnhanced } from "@/components/JobsBoardViewEnhanced";
import { jobStatuses, type JobStatus, type JobListSummary } from "@/lib/schema";
import { jobStatusPresentation } from "@/lib/ui";

// Mock data for the demo
const mockJobs: JobListSummary[] = [
  {
    job: {
      id: "1",
      title: "Senior Frontend Engineer",
      status: "wishlist" as JobStatus,
      appliedAt: null,
      notes: null,
      description: null,
      postingState: "active",
      lastCheckedAt: new Date().toISOString(),
      lastCheckResult: null,
      url: "https://example.com/job1",
      isNewFromWatch: false,
      isFavorite: true,
      appeal: 5,
      applicationEase: 4,
      salaryMin: 150000,
      salaryMax: 200000,
      location: "Remote, US",
      updatedAt: new Date(Date.now() - 86400000).toISOString(), // 1 day ago
    },
    companyName: "TechCorp Inc",
  },
  {
    job: {
      id: "2",
      title: "Product Designer",
      status: "applied" as JobStatus,
      appliedAt: new Date(Date.now() - 172800000).toISOString(), // 2 days ago
      notes: "Applied through referral",
      description: null,
      postingState: "active",
      lastCheckedAt: new Date().toISOString(),
      lastCheckResult: null,
      url: "https://example.com/job2",
      isNewFromWatch: false,
      isFavorite: false,
      appeal: 4,
      applicationEase: 3,
      salaryMin: 120000,
      salaryMax: 160000,
      location: "San Francisco, CA",
      updatedAt: new Date(Date.now() - 172800000).toISOString(),
    },
    companyName: "DesignStudio",
  },
  {
    job: {
      id: "3",
      title: "Backend Engineer",
      status: "interviewing" as JobStatus,
      appliedAt: new Date(Date.now() - 345600000).toISOString(), // 4 days ago
      notes: "Technical interview scheduled",
      description: null,
      postingState: "active",
      lastCheckedAt: new Date().toISOString(),
      lastCheckResult: null,
      url: "https://example.com/job3",
      isNewFromWatch: false,
      isFavorite: true,
      appeal: 5,
      applicationEase: 5,
      salaryMin: 140000,
      salaryMax: 180000,
      location: "New York, NY",
      updatedAt: new Date(Date.now() - 86400000).toISOString(),
    },
    companyName: "DataSystems",
  },
  {
    job: {
      id: "4",
      title: "Engineering Manager",
      status: "offer" as JobStatus,
      appliedAt: new Date(Date.now() - 604800000).toISOString(), // 7 days ago
      notes: "Offer received, negotiating",
      description: null,
      postingState: "active",
      lastCheckedAt: new Date().toISOString(),
      lastCheckResult: null,
      url: "https://example.com/job4",
      isNewFromWatch: false,
      isFavorite: true,
      appeal: 5,
      applicationEase: 2,
      salaryMin: 180000,
      salaryMax: 220000,
      location: "Remote",
      updatedAt: new Date(Date.now() - 172800000).toISOString(),
    },
    companyName: "ScaleUpTech",
  },
  {
    job: {
      id: "5",
      title: "UX Researcher",
      status: "rejected" as JobStatus,
      appliedAt: new Date(Date.now() - 1209600000).toISOString(), // 14 days ago
      notes: "Not enough experience",
      description: null,
      postingState: "inactive",
      lastCheckedAt: new Date().toISOString(),
      lastCheckResult: null,
      url: "https://example.com/job5",
      isNewFromWatch: false,
      isFavorite: false,
      appeal: 3,
      applicationEase: 4,
      salaryMin: 90000,
      salaryMax: 120000,
      location: "Austin, TX",
      updatedAt: new Date(Date.now() - 86400000).toISOString(),
    },
    companyName: "ResearchLabs",
  },
];

export function EnhancementsDemo() {
  const [jobs, setJobs] = useState<JobListSummary[]>(mockJobs);
  const [showKeyboardHelp, setShowKeyboardHelp] = useState(false);
  const [activeTab, setActiveTab] = useState<"quick-change" | "board" | "colors">("quick-change");

  const handleStatusChange = (jobId: string, newStatus: JobStatus) => {
    setJobs(prev => prev.map(job => 
      job.job.id === jobId 
        ? { ...job, job: { ...job.job, status: newStatus } }
        : job
    ));
    console.log(`Job ${jobId} status changed to ${newStatus}`);
  };

  const handleToggleFavorite = (jobId: string) => {
    setJobs(prev => prev.map(job =>
      job.job.id === jobId
        ? { ...job, job: { ...job.job, isFavorite: !job.job.isFavorite } }
        : job
    ));
  };

  const handleToggleArchive = (jobId: string, currentStatus: JobStatus) => {
    const newStatus = currentStatus === "archived" ? "wishlist" : "archived";
    handleStatusChange(jobId, newStatus);
  };

  return (
    <div className="mx-auto max-w-6xl space-y-8">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="font-display text-3xl font-semibold tracking-tight">
            Job Tracker Enhancements Demo
          </h1>
          <p className="mt-2 text-[var(--muted)]">
            Explore new ways to change job status with pop-up animations and richer colors
          </p>
        </div>
        <Link to="/" className="btn btn-secondary">
          ← Back to Jobs
        </Link>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-[var(--border)]">
        <button
          type="button"
          onClick={() => setActiveTab("quick-change")}
          className={`px-4 py-3 font-medium text-sm border-b-2 transition-colors ${
            activeTab === "quick-change"
              ? "border-[var(--accent)] text-[var(--accent-ink)]"
              : "border-transparent text-[var(--muted)] hover:text-[var(--foreground)]"
          }`}
        >
          Quick Status Change
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("board")}
          className={`px-4 py-3 font-medium text-sm border-b-2 transition-colors ${
            activeTab === "board"
              ? "border-[var(--accent)] text-[var(--accent-ink)]"
              : "border-transparent text-[var(--muted)] hover:text-[var(--foreground)]"
          }`}
        >
          Enhanced Board View
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("colors")}
          className={`px-4 py-3 font-medium text-sm border-b-2 transition-colors ${
            activeTab === "colors"
              ? "border-[var(--accent)] text-[var(--accent-ink)]"
              : "border-transparent text-[var(--muted)] hover:text-[var(--foreground)]"
          }`}
        >
          Color Palette
        </button>
      </div>

      {/* Quick Status Change Demo */}
      {activeTab === "quick-change" && (
        <div className="space-y-8">
          <section className="card p-6">
            <h2 className="font-display text-xl font-semibold mb-4">
              Quick Status Change Component
            </h2>
            <p className="text-[var(--muted)] mb-6">
              Click on any status badge to open a pop-up menu with all status options.
              Features animated feedback, keyboard navigation, and success indicators.
            </p>

            <div className="space-y-6">
              <div>
                <h3 className="font-medium text-sm text-[var(--muted)] mb-3">Different Sizes</h3>
                <div className="flex flex-wrap items-center gap-4">
                  <StatusQuickChange
                    currentStatus="wishlist"
                    onStatusChange={(status) => console.log("Changed to:", status)}
                    size="sm"
                  />
                  <StatusQuickChange
                    currentStatus="applied"
                    onStatusChange={(status) => console.log("Changed to:", status)}
                    size="md"
                  />
                  <StatusQuickChange
                    currentStatus="interviewing"
                    onStatusChange={(status) => console.log("Changed to:", status)}
                    size="lg"
                  />
                </div>
              </div>

              <div>
                <h3 className="font-medium text-sm text-[var(--muted)] mb-3">All Status Types</h3>
                <div className="flex flex-wrap gap-3">
                  {jobStatuses.map((status) => (
                    <StatusQuickChange
                      key={status}
                      currentStatus={status}
                      onStatusChange={(newStatus) => {
                        console.log(`Changed from ${status} to ${newStatus}`);
                      }}
                    />
                  ))}
                </div>
              </div>

              <div className="p-4 bg-[var(--surface-muted)] rounded-xl">
                <h4 className="font-medium mb-2">Features:</h4>
                <ul className="text-sm text-[var(--muted)] space-y-1">
                  <li>• <span className="text-[var(--foreground)]">Animated pop-up menu</span> with scale-in animation</li>
                  <li>• <span className="text-[var(--foreground)]">Success feedback</span> with checkmark and pulse animation</li>
                  <li>• <span className="text-[var(--foreground)]">Keyboard navigation</span> (arrow keys, Enter, Escape)</li>
                  <li>• <span className="text-[var(--foreground)]">Status icons</span> for visual recognition</li>
                  <li>• <span className="text-[var(--foreground)]">Accessible</span> with proper ARIA labels</li>
                </ul>
              </div>
            </div>
          </section>

          <section className="card p-6">
            <h2 className="font-display text-xl font-semibold mb-4">
              Keyboard Shortcuts
            </h2>
            <div className="space-y-4">
              <p className="text-[var(--muted)]">
                Press <kbd className="keyboard-shortcut">Ctrl</kbd> + <kbd className="keyboard-shortcut">/</kbd> to toggle keyboard shortcuts help.
                Use number keys <kbd className="keyboard-shortcut">1</kbd>-<kbd className="keyboard-shortcut">5</kbd> for quick status changes.
              </p>
              
              <button
                type="button"
                onClick={() => setShowKeyboardHelp(true)}
                className="btn btn-primary"
              >
                Show Keyboard Shortcuts Help
              </button>

              <div className="p-4 bg-gradient-to-r from-[var(--violet-soft)] to-[var(--blue-soft)] rounded-xl">
                <h4 className="font-medium mb-2 text-[var(--foreground)]">Quick Status Keys:</h4>
                <div className="flex flex-wrap gap-2">
                  <kbd className="keyboard-shortcut bg-[var(--violet-soft)] text-[var(--violet-ink)] border-[var(--violet-ink)]/30">1</kbd>
                  <span className="text-sm">Wishlist</span>
                  <kbd className="keyboard-shortcut bg-[var(--blue-soft)] text-[var(--blue-ink)] border-[var(--blue-ink)]/30">2</kbd>
                  <span className="text-sm">Applied</span>
                  <kbd className="keyboard-shortcut bg-[var(--amber-soft)] text-[var(--amber-ink)] border-[var(--amber-ink)]/30">3</kbd>
                  <span className="text-sm">Interviewing</span>
                  <kbd className="keyboard-shortcut bg-[var(--green-soft)] text-[var(--green-ink)] border-[var(--green-ink)]/30">4</kbd>
                  <span className="text-sm">Offer</span>
                  <kbd className="keyboard-shortcut bg-[var(--stone-soft)] text-[var(--stone-ink)] border-[var(--stone-ink)]/30">5</kbd>
                  <span className="text-sm">Rejected</span>
                </div>
              </div>
            </div>
          </section>
        </div>
      )}

      {/* Enhanced Board View Demo */}
      {activeTab === "board" && (
        <div className="space-y-6">
          <section className="card p-6">
            <h2 className="font-display text-xl font-semibold mb-4">
              Enhanced Board View with Drag & Drop
            </h2>
            <p className="text-[var(--muted)] mb-6">
              Drag jobs between columns to change their status. Features custom drag images,
              visual feedback, and success animations.
            </p>

            <JobsBoardViewEnhanced
              jobs={jobs.filter(j => !["rejected", "withdrawn", "closed", "archived"].includes(j.job.status))}
              onToggleFavorite={handleToggleFavorite}
              onUpdateStatus={handleStatusChange}
              onToggleArchive={handleToggleArchive}
              onDeleteJob={(job) => {
                console.log("Delete job:", job);
                setJobs(prev => prev.filter(j => j.job.id !== job.id));
              }}
            />
          </section>

          <section className="grid grid-cols-2 gap-6">
            <div className="card p-5">
              <h3 className="font-medium mb-3">Drag & Drop Features</h3>
              <ul className="text-sm text-[var(--muted)] space-y-2">
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Custom drag image with job details</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Visual drop targets with pulsating borders</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Success animation with status confirmation</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Global drag overlay with instructions</span>
                </li>
              </ul>
            </div>

            <div className="card p-5">
              <h3 className="font-medium mb-3">Board Enhancements</h3>
              <ul className="text-sm text-[var(--muted)] space-y-2">
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Quick status change on each job card</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Enhanced colors with gradients and glow</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Column-level status controls</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span>Interactive hover effects</span>
                </li>
              </ul>
            </div>
          </section>
        </div>
      )}

      {/* Color Palette Demo */}
      {activeTab === "colors" && (
        <div className="space-y-8">
          <section className="card p-6">
            <h2 className="font-display text-xl font-semibold mb-6">
              Enhanced Color Palette
            </h2>
            
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-8">
              {["violet", "blue", "amber", "green"].map((tone) => (
                <div key={tone} className="space-y-2">
                  <div className={`h-24 rounded-xl ${tone === 'violet' ? 'status-enhanced-wishlist' : 
                    tone === 'blue' ? 'status-enhanced-applied' :
                    tone === 'amber' ? 'status-enhanced-interviewing' :
                    'status-enhanced-offer'}`}>
                  </div>
                  <div className="text-center">
                    <p className="font-medium capitalize">{tone}</p>
                    <p className="text-xs text-[var(--muted)]">
                      {tone === 'violet' ? 'Wishlist' :
                       tone === 'blue' ? 'Applied' :
                       tone === 'amber' ? 'Interviewing' : 'Offer'}
                    </p>
                  </div>
                </div>
              ))}
            </div>

            <div className="space-y-4">
              <h3 className="font-medium">Color Features</h3>
              <ul className="text-sm text-[var(--muted)] space-y-2">
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span><span className="text-[var(--foreground)]">Gradient backgrounds</span> for richer visual depth</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span><span className="text-[var(--foreground)]">Enhanced ink colors</span> for better contrast</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span><span className="text-[var(--foreground)]">Subtle glow effects</span> for visual emphasis</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-[var(--accent)] mt-0.5">•</span>
                  <span><span className="text-[var(--foreground)]">Dark mode optimized</span> with adjusted gradients</span>
                </li>
              </ul>
            </div>
          </section>

          <section className="card p-6">
            <h3 className="font-medium mb-4">Status Color Comparison</h3>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-[var(--border)]">
                    <th className="text-left py-3 font-medium">Status</th>
                    <th className="text-left py-3 font-medium">Original</th>
                    <th className="text-left py-3 font-medium">Enhanced</th>
                    <th className="text-left py-3 font-medium">Features</th>
                  </tr>
                </thead>
                <tbody>
                  {jobStatuses.map((status) => {
                    const info = jobStatusPresentation(status);
                    return (
                      <tr key={status} className="border-b border-[var(--border)] last:border-0">
                        <td className="py-3">
                          <div className="flex items-center gap-2">
                            <span className={`w-3 h-3 rounded ${info.tone === 'violet' ? 'bg-[var(--violet-ink)]' : 
                              info.tone === 'blue' ? 'bg-[var(--blue-ink)]' :
                              info.tone === 'amber' ? 'bg-[var(--amber-ink)]' :
                              info.tone === 'green' ? 'bg-[var(--green-ink)]' :
                              'bg-[var(--stone-ink)]'}`}></span>
                            <span>{info.label}</span>
                          </div>
                        </td>
                        <td className="py-3">
                          <div className={`px-3 py-1 rounded text-xs ${toneClasses[info.tone]} inline-block`}>
                            Original
                          </div>
                        </td>
                        <td className="py-3">
                          <div className={`px-3 py-1 rounded text-xs ${
                            info.tone === 'violet' ? 'status-enhanced-wishlist' : 
                            info.tone === 'blue' ? 'status-enhanced-applied' :
                            info.tone === 'amber' ? 'status-enhanced-interviewing' :
                            info.tone === 'green' ? 'status-enhanced-offer' :
                            'status-enhanced-stone'
                          } inline-block`}>
                            Enhanced
                          </div>
                        </td>
                        <td className="py-3 text-[var(--muted)]">
                          {info.tone !== 'stone' && info.tone !== 'danger' ? (
                            <span>Gradient + Glow</span>
                          ) : (
                            <span>Gradient</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </section>
        </div>
      )}

      {/* Footer */}
      <div className="card p-6 mt-8">
        <h3 className="font-medium mb-3">Implementation Notes</h3>
        <ul className="text-sm text-[var(--muted)] space-y-2">
          <li className="flex items-start gap-2">
            <span className="text-[var(--accent)] mt-0.5">•</span>
            <span>All components are designed to work with the existing job tracker architecture</span>
          </li>
          <li className="flex items-start gap-2">
            <span className="text-[var(--accent)] mt-0.5">•</span>
            <span>CSS animations are optimized with <code className="text-xs">prefers-reduced-motion</code> support</span>
          </li>
          <li className="flex items-start gap-2">
            <span className="text-[var(--accent)] mt-0.5">•</span>
            <span>Accessibility features include keyboard navigation, ARIA labels, and focus management</span>
          </li>
          <li className="flex items-start gap-2">
            <span className="text-[var(--accent)] mt-0.5">•</span>
            <span>Components can be incrementally adopted without breaking existing functionality</span>
          </li>
        </ul>
      </div>

      {/* Keyboard Shortcuts Modal */}
      <KeyboardShortcutsHelp
        isOpen={showKeyboardHelp}
        onClose={() => setShowKeyboardHelp(false)}
      />
    </div>
  );
}