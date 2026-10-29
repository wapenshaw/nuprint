import type { AdapterStatus } from "../bindings/AdapterStatus";
import type { Density } from "../bindings/Density";
import type { JobState } from "../bindings/JobState";
import type { StatusFlag } from "../bindings/StatusFlag";
import type { SupportLevel } from "../bindings/SupportLevel";

export type BadgeTone = "neutral" | "active" | "good" | "warn" | "bad";

export const JOB_STATE_LABEL: Record<JobState, string> = {
  held: "Held",
  queued: "Waiting",
  connecting: "Connecting",
  sending: "Sending",
  waiting_for_ready: "Finishing",
  completed: "Done · printer ready",
  transfer_unconfirmed: "Sent · output unconfirmed",
  accepted: "Accepted",
  failed: "Failed",
  uncertain: "Uncertain — check paper",
  cancelled: "Cancelled",
};

export const JOB_STATE_TONE: Record<JobState, BadgeTone> = {
  held: "neutral",
  queued: "neutral",
  connecting: "active",
  sending: "active",
  waiting_for_ready: "active",
  completed: "good",
  transfer_unconfirmed: "warn",
  accepted: "warn",
  failed: "bad",
  uncertain: "warn",
  cancelled: "neutral",
};

const TERMINAL: ReadonlySet<JobState> = new Set([
  "completed",
  "transfer_unconfirmed",
  "accepted",
  "failed",
  "uncertain",
  "cancelled",
]);
const CANCELLABLE: ReadonlySet<JobState> = new Set([
  "held",
  "queued",
  "connecting",
  "sending",
  "waiting_for_ready",
]);
const ACTIVE: ReadonlySet<JobState> = new Set(["connecting", "sending", "waiting_for_ready"]);

export const isTerminal = (s: JobState) => TERMINAL.has(s);
export const isCancellable = (s: JobState) => CANCELLABLE.has(s);
export const isActive = (s: JobState) => ACTIVE.has(s);
/** Jobs that still count toward the queue badge. */
export const isOpenJob = (s: JobState) => !TERMINAL.has(s);

export const SUPPORT_LABEL: Record<SupportLevel, string> = {
  candidate: "Not supported",
  experimental: "Experimental",
  verified_basic: "Verified",
  release_supported: "Supported",
};

export const SUPPORT_TONE: Record<SupportLevel, BadgeTone> = {
  candidate: "bad",
  experimental: "warn",
  verified_basic: "good",
  release_supported: "good",
};

export const DENSITY_LABEL: Record<Density, string> = {
  light: "Light",
  normal: "Normal",
  dark: "Dark",
};

export const FLAG_LABEL: Record<StatusFlag, string> = {
  paper_out: "Out of paper",
  cover_open: "Cover open",
  overheated: "Overheated",
  low_power: "Low battery",
  charging: "Charging",
  printing: "Printing",
};

export const ADAPTER_MESSAGE: Record<AdapterStatus, { tone: BadgeTone; label: string; text: string }> = {
  ready: { tone: "good", label: "Bluetooth ready", text: "Bluetooth is on and ready to find printers." },
  powered_off: {
    tone: "warn",
    label: "Bluetooth is off",
    text: "Bluetooth is turned off on this computer. Turn it on in your system settings, then find printers again.",
  },
  absent: {
    tone: "bad",
    label: "No Bluetooth adapter",
    text: "No Bluetooth adapter was found on this computer. Connect a Bluetooth adapter, then check again.",
  },
  permission_denied: {
    tone: "bad",
    label: "Bluetooth permission needed",
    text: "NuPrint is not allowed to use Bluetooth. Allow Bluetooth access for NuPrint in your system's privacy settings, then check again.",
  },
  permission_not_asked: {
    tone: "neutral",
    label: "Bluetooth permission",
    text: "When you press Find printers, macOS will ask whether NuPrint may use Bluetooth. Choose Allow so NuPrint can find your printer.",
  },
  permission_pending: {
    tone: "warn",
    label: "Waiting for Bluetooth permission",
    text: "macOS is asking whether NuPrint may use Bluetooth. Choose Allow in that prompt. If you can't see it, allow NuPrint in Bluetooth privacy settings.",
  },
  unknown: {
    tone: "neutral",
    label: "Bluetooth status unknown",
    text: "Bluetooth status is not known yet. Try finding printers to find out.",
  },
};
