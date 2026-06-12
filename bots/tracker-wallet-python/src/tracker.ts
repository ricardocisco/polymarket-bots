import { Client, EmbedBuilder } from "discord.js";
import { Wallet, Subscription } from "./models.js";
import { fetchRecentActivity, getUsernameFromAddress } from "./polymarket.js";
import { logger } from "./logger.js";

const CHECK_INTERVAL = 10000; // 10 segundos

const ENTRY_HIGH_USD = 1000;
const ENTRY_MEDIUM_USD = 500;
const ENTRY_LOW_USD = 10;

function getEntryTier(valueUsd: number): {
  label: string;
  emoji: string;
} {
  if (valueUsd >= ENTRY_HIGH_USD) {
    return { label: "ALTA", emoji: "🚀" };
  }

  if (valueUsd >= ENTRY_MEDIUM_USD) {
    return { label: "MEDIANA", emoji: "📈" };
  }

  if (valueUsd < ENTRY_LOW_USD) {
    return { label: "BAIXA", emoji: "🧊" };
  }

  return { label: "MÉDIA", emoji: "➖" };
}

// Cache para evitar duplicação de mensagens
const sentMessages = new Map<string, number>();
const MESSAGE_CACHE_TTL = 120000; // 2 minutos

export async function startTrackerLoop(client: Client) {
  logger.info(`🔥 TRACKER V4 INICIADO`);
  logger.info(`🎯 Detecta: Novas posições, aumentos, diminuições e fechamentos`);
  logger.info(`⏱️  Intervalo: ${CHECK_INTERVAL / 1000}s\n`);

  // Aguarda o bot estar pronto
  if (!client.isReady()) {
    logger.info(`⏳ Aguardando bot ficar online...`);
    await new Promise((resolve) => {
      client.once("clientReady", resolve);
    });
    logger.info(`✅ Bot online! Iniciando monitoramento...\n`);
  }

  setInterval(async () => {
    try {
      const timestamp = new Date().toLocaleTimeString("pt-BR");
      logger.debug(`\n💓 [${timestamp}] Verificando carteiras...`);

      // Limpa cache de mensagens antigas
      const now = Date.now();
      for (const [id, timestamp] of sentMessages.entries()) {
        if (now - timestamp > MESSAGE_CACHE_TTL) {
          sentMessages.delete(id);
        }
      }

      const wallets = await Wallet.find();
      logger.debug(`📊 Total de carteiras monitoradas: ${wallets.length}`);

      if (wallets.length === 0) {
        logger.debug(`⚠️ Nenhuma carteira cadastrada`);
        return;
      }

      for (const wallet of wallets) {
        if (!wallet.address.startsWith("0x")) {
          logger.warn(`⚠️ Endereço inválido: ${wallet.address}`);
          continue;
        }

        // Verifica se tem inscrições ativas
        const subs = await Subscription.find({ walletAddress: wallet.address });
        if (subs.length === 0) {
          logger.debug(`⚠️ Carteira ${wallet.address.slice(0, 8)} sem inscrições ativas`);
          continue;
        }

        logger.debug(`🔍 Checando ${wallet.address.slice(0, 8)}... (${subs.length} canal(is))`);

        // Busca atividades via /activity endpoint (desde o último timestamp)
        const activities = await fetchRecentActivity(wallet.address, wallet.lastTimestamp);
        const trackedDisplayName = await getUsernameFromAddress(wallet.address);

        if (activities.length === 0) {
          // Não loga mais nada aqui, o fetchRecentActivity já loga
          continue;
        }

        logger.info(`🚨 MUDANÇA DETECTADA: ${activities.length} operação(ões)\n`);

        // Atualiza timestamp com o maior timestamp recebido (converte s → ms para compatibilidade)
        const maxTs = Math.max(...activities.map((a) => a.timestamp)) * 1000;
        await Wallet.updateOne({ _id: wallet._id }, { lastTimestamp: maxTs });
        // Processa cada trade detectado
        for (const trade of activities) {
          // Verifica se já enviou
          if (sentMessages.has(trade.id)) {
            logger.debug(`   ⏭️ Pulando duplicata: ${trade.id.slice(0, 20)}...`);
            continue;
          }

          logger.debug(`\n   📤 Preparando mensagem...`);
          logger.debug(`      Tipo: ${trade.side}`);
          logger.debug(`      Mercado: ${trade.marketTitle.slice(0, 60)}`);
          logger.debug(`      Outcome: ${trade.outcome}`);
          logger.debug(`      Shares: ${trade.amount.toFixed(1)}`);
          logger.debug(`      Preço: ${trade.price.toFixed(3)}`);
          logger.debug(`      EventSlug: ${trade.eventSlug || "(sem eventSlug)"}`);
          logger.debug(`      MarketSlug: ${trade.marketSlug || "(sem marketSlug)"}`);

          // Detecção de tipo e cor
          let typeLabel = "OPERAÇÃO";
          let color = 0x808080;
          let emoji = "📊";

          const side = (trade.side || "").toUpperCase();

          if (side === "BUY") {
            typeLabel = "COMPROU";
            color = 0x00ff00;
            emoji = "🟢";
          } else if (side === "SELL") {
            typeLabel = "VENDEU";
            color = 0xff0000;
            emoji = "🔴";
          }

          // URL do mercado: pré-construída pela /activity a partir dos slugs
          const marketUrl = trade.marketUrl || `https://polymarket.com/profile/${wallet.address}`;
          const marketTitle = trade.marketUrl ? `[${trade.marketTitle}](${trade.marketUrl})` : trade.marketTitle;

          // Monta descrição
          let description = "";

          // Menciona o usuário que criou o tracking (se disponível)
          const trackingUserIds = new Set<string>();
          for (const sub of subs) {
            if (sub.userId) {
              trackingUserIds.add(sub.userId);
            }
          }

          // Adiciona menções dos usuários que criaram o tracking
          // if (trackingUserIds.size > 0) {
          //   const mentions = Array.from(trackingUserIds)
          //     .map((uid) => `<@${uid}>`)
          //     .join(" ");
          //   description += `${mentions}\n\n`;
          // }

          const traderName = trackedDisplayName || trade.displayName;
          if (traderName) {
            description += `**Trader:** ${traderName}\n`;
          }

          // Usa o marketTitle que já tem o link (ou não)
          description += `**Mercado:** ${marketTitle}\n`;
          description += `**Posição:** ${trade.outcome}\n`;
          description += `**Carteira:** [\`${wallet.address}\`](https://polymarket.com/profile/${wallet.address})`;

          const tradeValue = trade.usdcSize || trade.price * trade.amount;
          const entryTier = getEntryTier(tradeValue);

          // Cria o embed
          const embed = new EmbedBuilder()
            .setTitle(`${emoji} ${typeLabel}`)
            .setURL(marketUrl)
            .setColor(color)
            .setDescription(description)
            .addFields(
              {
                name: `${entryTier.emoji} Entrada`,
                value: `${entryTier.label} ($${tradeValue.toFixed(2)})`,
                inline: true
              },
              {
                name: "💵 Preço",
                value: `$${trade.price.toFixed(3)}`,
                inline: true
              },
              {
                name: "📊 Shares",
                value: `${trade.amount.toFixed(1)}`,
                inline: true
              },
              {
                name: "💰 Valor",
                value: `$${tradeValue.toFixed(2)}`,
                inline: true
              }
            )
            .setFooter({ text: `Polymarket Tracker` })
            .setTimestamp(new Date(trade.timestamp * 1000));

          // P&L realizado (apenas em vendas)
          if (trade.realizedPnl !== undefined && trade.side === "SELL") {
            if (trade.realizedPnl > 0) {
              embed.addFields({ name: "🏆 Profit", value: `$${trade.realizedPnl.toFixed(2)}`, inline: true });
            } else if (trade.realizedPnl < 0) {
              embed.addFields({ name: "📉 Loss", value: `-$${Math.abs(trade.realizedPnl).toFixed(2)}`, inline: true });
            }
          }

          // Adiciona imagem do mercado se disponível
          if (trade.marketImageUrl && trade.marketImageUrl.length > 0) {
            embed.setImage(trade.marketImageUrl);
          }

          // Envia para todos os canais inscritos
          let sentCount = 0;
          let filteredCount = 0;
          let failedCount = 0;
          for (const sub of subs) {
            // --- Logica de filtros ---
            if (sub.filters) {
              const { minUsd, keywords } = sub.filters;

              if (minUsd && minUsd > 0 && tradeValue < minUsd) {
                filteredCount++;
                logger.debug(`   Canal ${sub.channelId} filtrado por valor ($${tradeValue.toFixed(2)} < $${minUsd})`);
                continue;
              }

              if (keywords && keywords.length > 0) {
                const titleLower = trade.marketTitle.toLowerCase();
                const hasKeyword = keywords.some((k) => titleLower.includes(k.toLowerCase()));
                if (!hasKeyword) {
                  filteredCount++;
                  logger.debug(`   Canal ${sub.channelId} filtrado por keyword: "${trade.marketTitle}"`);
                  continue;
                }
              }
            }

            try {
              const channel = await client.channels.fetch(sub.channelId).catch((error: any) => {
                logger.error(
                  `   Erro ao buscar canal ${sub.channelId}: ${error?.code ?? "UNKNOWN"} ${error?.message ?? error}`
                );
                return null;
              });

              if (!channel) {
                failedCount++;
                logger.error(`   Canal ${sub.channelId} nao encontrado ou inacessivel pelo bot`);
                continue;
              }

              if (!channel.isTextBased() || !("send" in channel) || typeof channel.send !== "function") {
                failedCount++;
                logger.error(`   Canal ${sub.channelId} nao aceita envio de mensagens`);
                continue;
              }

              const sendableChannel = channel as typeof channel & {
                send: (options: { embeds: EmbedBuilder[] }) => Promise<unknown>;
              };

              await sendableChannel.send({ embeds: [embed] });
              sentCount++;
              logger.info(`   Enviado para canal ${sub.channelId}`);
            } catch (e: any) {
              failedCount++;
              logger.error(`   Erro ao enviar para ${sub.channelId}: ${e?.code ?? "UNKNOWN"} ${e?.message ?? e}`);
            }
          }

          if (sentCount > 0) {
            // Marca como enviado
            sentMessages.set(trade.id, Date.now());
            logger.info(`   Mensagem enviada com sucesso para ${sentCount} canal(is)\n`);
          } else {
            logger.warn(
              `   Nenhum canal recebeu a mensagem (filtrados=${filteredCount}, falhas=${failedCount}, inscritos=${subs.length})\n`
            );
          }

          // Delay entre mensagens
          await new Promise((r) => setTimeout(r, 500));
        }

        // Pausa entre carteiras
        await new Promise((r) => setTimeout(r, 1000));
      }

      logger.debug(`✓ Ciclo de verificação concluído\n`);
    } catch (e) {
      logger.error("❌ Erro no Loop do Tracker:", e);
    }
  }, CHECK_INTERVAL);
}
