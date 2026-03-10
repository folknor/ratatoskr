export function buildQuote(msg: {
  from_name: string | null;
  from_address: string | null;
  date: string | number;
  body_html: string | null;
  body_text: string | null;
}): string {
  const date = new Date(msg.date).toLocaleString();
  const from = msg.from_name
    ? `${msg.from_name} &lt;${msg.from_address}&gt;`
    : (msg.from_address ?? "Unknown");
  return `<br><br><div style="border-left:2px solid #ccc;padding-left:12px;margin-left:0;color:#666">On ${date}, ${from} wrote:<br>${msg.body_html ?? msg.body_text ?? ""}</div>`;
}

export function buildForwardQuote(msg: {
  from_name: string | null;
  from_address: string | null;
  date: string | number;
  subject: string | null;
  to_addresses: string | null;
  body_html: string | null;
  body_text: string | null;
}): string {
  const date = new Date(msg.date).toLocaleString();
  return `<br><br>---------- Forwarded message ---------<br>From: ${msg.from_name ?? ""} &lt;${msg.from_address ?? ""}&gt;<br>Date: ${date}<br>Subject: ${msg.subject ?? ""}<br>To: ${msg.to_addresses ?? ""}<br><br>${msg.body_html ?? msg.body_text ?? ""}`;
}
