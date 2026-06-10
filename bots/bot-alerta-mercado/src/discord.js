export async function sendDiscordWebhook(webhookUrl, payload) {
  if (!webhookUrl) {
    throw new Error('DISCORD_WEBHOOK_URL nao configurado.');
  }

  const response = await fetch(webhookUrl, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
    },
    body: JSON.stringify(payload),
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`Discord webhook retornou HTTP ${response.status}: ${body.slice(0, 300)}`);
  }
}
