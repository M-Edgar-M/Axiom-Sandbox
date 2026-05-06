/**
 * SettingsModal — Secure API Credential Entry
 *
 * Security contract:
 *  - Keys are held ONLY in local component state (never localStorage / context).
 *  - On success the state is immediately zeroed out.
 *  - The Rust backend owns all persistence (writes to .env on disk).
 */

import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

// ─── Toast ────────────────────────────────────────────────────────────────────

type ToastVariant = "success" | "error";

interface ToastProps {
  message: string;
  variant: ToastVariant;
  onDismiss: () => void;
}

function Toast({ message, variant, onDismiss }: ToastProps) {
  useEffect(() => {
    const t = setTimeout(onDismiss, 4000);
    return () => clearTimeout(t);
  }, [onDismiss]);

  const isSuccess = variant === "success";
  return (
    <div
      className={`fixed bottom-6 right-6 z-[200] flex items-center gap-3 px-5 py-3 rounded-lg shadow-2xl border transition-all animate-fade-in
        ${isSuccess
          ? "bg-[#0A1A12] border-[#00FFAA]/30 text-[#00FFAA]"
          : "bg-[#1A0A0A] border-red-500/30 text-red-400"
        }`}
    >
      <span className="material-symbols-outlined text-[20px]">
        {isSuccess ? "check_circle" : "error"}
      </span>
      <span className="text-sm font-medium">{message}</span>
      <button
        onClick={onDismiss}
        className="ml-2 opacity-60 hover:opacity-100 transition-opacity"
        aria-label="Dismiss notification"
      >
        <span className="material-symbols-outlined text-[16px]">close</span>
      </button>
    </div>
  );
}

// ─── Password Field ───────────────────────────────────────────────────────────

interface PasswordFieldProps {
  id: string;
  label: string;
  placeholder: string;
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
  autoComplete?: string;
}

function PasswordField({
  id,
  label,
  placeholder,
  value,
  onChange,
  disabled,
  autoComplete = "off",
}: PasswordFieldProps) {
  const [visible, setVisible] = useState(false);

  return (
    <div className="flex flex-col gap-1.5">
      <label
        htmlFor={id}
        className="text-xs font-medium tracking-widest uppercase text-gray-400"
      >
        {label}
      </label>
      <div className="relative">
        <input
          id={id}
          type={visible ? "text" : "password"}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          autoComplete={autoComplete}
          spellCheck={false}
          className={`w-full bg-[#0D0D0D] border rounded-lg px-4 py-3 pr-12 text-sm font-mono
            text-white placeholder-gray-600 outline-none transition-all
            focus:border-[#0070FF]/60 focus:ring-1 focus:ring-[#0070FF]/30
            disabled:opacity-40 disabled:cursor-not-allowed
            ${value ? "border-[#0070FF]/30" : "border-white/10"}`}
        />
        <button
          type="button"
          tabIndex={-1}
          onClick={() => setVisible((v) => !v)}
          disabled={disabled}
          aria-label={visible ? "Hide key" : "Reveal key"}
          className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-500 hover:text-gray-300 transition-colors"
        >
          <span className="material-symbols-outlined text-[18px]">
            {visible ? "visibility_off" : "visibility"}
          </span>
        </button>
      </div>
    </div>
  );
}

// ─── Modal ────────────────────────────────────────────────────────────────────

export interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ message: string; variant: ToastVariant } | null>(null);

  // Focus the first field when the modal opens.
  const firstFieldRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (open) {
      // Small delay so CSS transition doesn't interfere with focus.
      const t = setTimeout(() => firstFieldRef.current?.focus(), 80);
      return () => clearTimeout(t);
    }
    // Wipe any partial input if the modal is closed without saving.
    setApiKey("");
    setApiSecret("");
  }, [open]);

  // Close on Escape key.
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  async function handleSave(e: React.FormEvent) {
    e.preventDefault();
    if (!apiKey.trim() || !apiSecret.trim()) {
      setToast({ message: "Both fields are required.", variant: "error" });
      return;
    }

    setSaving(true);
    try {
      await invoke("save_api_credentials", {
        apiKey: apiKey.trim(),
        apiSecret: apiSecret.trim(),
      });

      // ── Security: zero-out state immediately ─────────────────────────────
      setApiKey("");
      setApiSecret("");

      setToast({ message: "API credentials saved & loaded. Session ready.", variant: "success" });
      onClose();
    } catch (err) {
      setToast({
        message: typeof err === "string" ? err : "Failed to save credentials.",
        variant: "error",
      });
    } finally {
      setSaving(false);
    }
  }

  if (!open) {
    return toast ? (
      <Toast
        message={toast.message}
        variant={toast.variant}
        onDismiss={() => setToast(null)}
      />
    ) : null;
  }

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-[100] bg-black/70 backdrop-blur-sm"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Dialog */}
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-modal-title"
        className="fixed z-[110] inset-0 flex items-center justify-center p-4 pointer-events-none"
      >
        <div className="pointer-events-auto w-full max-w-md bg-[#111111] border border-white/10 rounded-2xl shadow-[0_24px_64px_rgba(0,0,0,0.8)] overflow-hidden">
          {/* Header */}
          <div className="flex items-center justify-between px-6 pt-6 pb-4 border-b border-white/5">
            <div className="flex items-center gap-3">
              <div className="w-9 h-9 rounded-lg bg-[#0070FF]/10 border border-[#0070FF]/20 flex items-center justify-center">
                <span className="material-symbols-outlined text-[20px] text-[#0070FF]">key</span>
              </div>
              <div>
                <h2
                  id="settings-modal-title"
                  className="text-white font-semibold text-sm tracking-wide"
                >
                  API Credentials
                </h2>
                <p className="text-gray-500 text-xs mt-0.5">Binance Testnet / Live</p>
              </div>
            </div>
            <button
              id="settings-modal-close"
              onClick={onClose}
              aria-label="Close settings"
              className="w-8 h-8 flex items-center justify-center rounded-lg text-gray-500 hover:text-white hover:bg-white/5 transition-colors"
            >
              <span className="material-symbols-outlined text-[20px]">close</span>
            </button>
          </div>

          {/* Body */}
          <form onSubmit={handleSave} autoComplete="off" className="px-6 py-5 flex flex-col gap-5">
            {/* Security notice */}
            <div className="flex items-start gap-3 bg-amber-500/5 border border-amber-500/20 rounded-lg px-4 py-3">
              <span className="material-symbols-outlined text-[18px] text-amber-400 mt-0.5 shrink-0">
                shield_lock
              </span>
              <p className="text-amber-300/80 text-xs leading-relaxed">
                Keys are passed directly to the Rust backend and written to your local{" "}
                <code className="font-mono text-amber-200">.env</code> file.
                They are{" "}
                <strong className="text-amber-200">never stored in the browser</strong> or
                transmitted to any external server.
              </p>
            </div>

            {/* Fields */}
            <div className="flex flex-col gap-4">
              {/* Attach ref to the underlying input via a wrapper trick */}
              <div ref={(el) => { if (el) firstFieldRef.current = el.querySelector("input"); }}>
                <PasswordField
                  id="settings-api-key"
                  label="Binance API Key"
                  placeholder="Paste your API key…"
                  value={apiKey}
                  onChange={setApiKey}
                  disabled={saving}
                  autoComplete="new-password"
                />
              </div>

              <PasswordField
                id="settings-api-secret"
                label="Binance API Secret"
                placeholder="Paste your API secret…"
                value={apiSecret}
                onChange={setApiSecret}
                disabled={saving}
                autoComplete="new-password"
              />
            </div>

            {/* Strength indicators */}
            <div className="flex gap-3">
              {[
                { label: "API Key", value: apiKey },
                { label: "API Secret", value: apiSecret },
              ].map(({ label, value }) => {
                const filled = value.trim().length > 0;
                return (
                  <div key={label} className="flex items-center gap-1.5 text-xs text-gray-500">
                    <span
                      className={`w-1.5 h-1.5 rounded-full transition-colors ${
                        filled ? "bg-[#00FFAA]" : "bg-gray-700"
                      }`}
                    />
                    {label} {filled ? "entered" : "empty"}
                  </div>
                );
              })}
            </div>

            {/* Actions */}
            <div className="flex items-center gap-3 pt-1">
              <button
                type="button"
                id="settings-cancel-btn"
                onClick={onClose}
                disabled={saving}
                className="flex-1 px-4 py-2.5 rounded-lg border border-white/10 text-gray-400 text-sm font-medium
                  hover:bg-white/5 hover:text-white transition-colors disabled:opacity-40"
              >
                Cancel
              </button>
              <button
                type="submit"
                id="settings-save-btn"
                disabled={saving || !apiKey.trim() || !apiSecret.trim()}
                className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg
                  bg-[#0070FF] text-white text-sm font-semibold
                  hover:bg-[#0060dd] active:scale-[0.98]
                  disabled:opacity-40 disabled:cursor-not-allowed
                  transition-all shadow-[0_4px_16px_rgba(0,112,255,0.3)]"
              >
                {saving ? (
                  <>
                    <span className="material-symbols-outlined text-[16px] animate-spin">
                      progress_activity
                    </span>
                    Saving…
                  </>
                ) : (
                  <>
                    <span className="material-symbols-outlined text-[16px]">save</span>
                    Save & Connect
                  </>
                )}
              </button>
            </div>

            <p className="text-center text-xs text-gray-600 -mt-1">
              Changes take effect immediately — no app restart required.
            </p>
          </form>
        </div>
      </div>

      {/* Toast rendered on top of the modal if needed */}
      {toast && (
        <Toast
          message={toast.message}
          variant={toast.variant}
          onDismiss={() => setToast(null)}
        />
      )}
    </>
  );
}
