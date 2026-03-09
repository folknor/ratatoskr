import type React from "react";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";
import { Modal } from "@/components/ui/Modal";
import { TextField } from "@/components/ui/TextField";
import type { DbCalendar } from "@/services/db/calendars";

interface EventCreateModalProps {
  calendars?: DbCalendar[];
  onClose: () => void;
  onCreate: (event: {
    summary: string;
    description: string;
    location: string;
    startTime: string;
    endTime: string;
    calendarId?: string | undefined;
  }) => void;
}

export function EventCreateModal({
  calendars,
  onClose,
  onCreate,
}: EventCreateModalProps): React.ReactNode {
  const { t } = useTranslation("calendar");
  const [summary, setSummary] = useState("");
  const [description, setDescription] = useState("");
  const [location, setLocation] = useState("");
  const [startTime, setStartTime] = useState(getDefaultStart());
  const [endTime, setEndTime] = useState(getDefaultEnd());
  const [calendarId, setCalendarId] = useState<string>(
    calendars?.find((c) => c.is_primary)?.id ?? calendars?.[0]?.id ?? "",
  );

  const handleSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      if (!summary.trim()) return;
      onCreate({
        summary: summary.trim(),
        description,
        location,
        startTime,
        endTime,
        calendarId: calendarId || undefined,
      });
    },
    [summary, description, location, startTime, endTime, calendarId, onCreate],
  );

  return (
    <Modal
      isOpen={true}
      onClose={onClose}
      title={t("createEvent")}
      width="w-full max-w-md"
    >
      <form onSubmit={handleSubmit} className="p-4 space-y-3">
        <TextField
          label={t("title")}
          type="text"
          value={summary}
          // biome-ignore lint/nursery/useExplicitType: inline callback
          onChange={(e) => setSummary(e.target.value)}
          placeholder={t("eventTitle")}
          autoFocus
        />

        {calendars && calendars.length > 1 && (
          <div>
            <label
              className="text-xs text-text-secondary block mb-1"
              htmlFor="calendar-select"
            >
              {t("calendar")}
            </label>
            <select
              id="calendar-select"
              value={calendarId}
              // biome-ignore lint/nursery/useExplicitType: inline callback
              onChange={(e) => setCalendarId(e.target.value)}
              className="w-full px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
            >
              {calendars.map((cal) => (
                <option key={cal.id} value={cal.id}>
                  {cal.display_name ?? t("calendar")}
                  {cal.is_primary ? ` ${t("calendarPrimary")}` : ""}
                </option>
              ))}
            </select>
          </div>
        )}

        <div className="grid grid-cols-2 gap-3">
          <TextField
            label={t("start")}
            type="datetime-local"
            value={startTime}
            // biome-ignore lint/nursery/useExplicitType: inline callback
            onChange={(e) => setStartTime(e.target.value)}
          />
          <TextField
            label={t("end")}
            type="datetime-local"
            value={endTime}
            // biome-ignore lint/nursery/useExplicitType: inline callback
            onChange={(e) => setEndTime(e.target.value)}
          />
        </div>

        <TextField
          label={t("location")}
          type="text"
          value={location}
          // biome-ignore lint/nursery/useExplicitType: inline callback
          onChange={(e) => setLocation(e.target.value)}
          placeholder={t("addLocation")}
        />

        <div>
          <label
            className="text-xs text-text-secondary block mb-1"
            htmlFor="event-description"
          >
            {t("description")}
          </label>
          <textarea
            id="event-description"
            value={description}
            // biome-ignore lint/nursery/useExplicitType: inline callback
            onChange={(e) => setDescription(e.target.value)}
            placeholder={t("addDescription")}
            rows={3}
            className="w-full px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent resize-none"
          />
        </div>

        <div className="flex justify-end gap-2 pt-2">
          <Button type="button" variant="secondary" size="md" onClick={onClose}>
            {t("cancel")}
          </Button>
          <Button
            type="submit"
            variant="primary"
            size="md"
            disabled={!summary.trim()}
          >
            {t("create")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

function getDefaultStart(): string {
  const now = new Date();
  now.setMinutes(0, 0, 0);
  now.setHours(now.getHours() + 1);
  return toLocalISOString(now);
}

function getDefaultEnd(): string {
  const now = new Date();
  now.setMinutes(0, 0, 0);
  now.setHours(now.getHours() + 2);
  return toLocalISOString(now);
}

function toLocalISOString(date: Date): string {
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
