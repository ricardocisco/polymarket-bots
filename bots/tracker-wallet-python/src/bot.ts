import { Client, GatewayIntentBits, EmbedBuilder, type Interaction } from "discord.js";
import { Wallet, Subscription } from "./models.js";
import { resolveUser, getUsernameFromAddress } from "./polymarket.js";
import { logger } from "./logger.js";

export const client = new Client({
  intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages]
});

client.on("interactionCreate", async (interaction: Interaction) => {
  if (!interaction.isChatInputCommand()) return;

  const rawInput = interaction.options.getString("input") || interaction.options.getString("carteira");

  // ===== COMANDO /TRACK =====
  if (interaction.commandName === "track") {
    await interaction.deferReply();

    if (!rawInput) {
      await interaction.editReply("❌ Você precisa fornecer um endereço ou @username.");
      return;
    }

    logger.info(`🔍 Tentando rastrear: ${rawInput}`);

    // Resolve o endereço 0x
    const address = await resolveUser(rawInput);

    if (!address) {
      await interaction.editReply(
        `❌ Não consegui encontrar o endereço para **${rawInput}**.\n` +
          `Certifique-se de que:\n` +
          `• O username está correto (ex: @nickname)\n` +
          `• Ou use o endereço 0x completo da carteira`
      );
      return;
    }

    try {
      // Busca ou cria a carteira
      let wallet = await Wallet.findOne({ address });

      if (!wallet) {
        logger.info(`🆕 Nova carteira: ${address}`);

        // IMPORTANTE: Define lastTimestamp como AGORA para só pegar trades FUTUROS
        wallet = await Wallet.create({
          address,
          lastTimestamp: Date.now()
        });

        logger.info(`   └─ Criada com timestamp: ${new Date(wallet.lastTimestamp).toISOString()}`);
      } else {
        logger.info(`♻️ Carteira já existe: ${address}`);
      }

      // Verifica se este canal já rastreia essa carteira
      const existingSub = await Subscription.findOne({
        channelId: interaction.channelId,
        walletAddress: address
      });

      if (existingSub) {
        await interaction.editReply(
          `⚠️ Este canal já está rastreando a carteira:\n` +
            `[\`${address.slice(0, 6)}...${address.slice(-4)}\`](https://polymarket.com/profile/${address})`
        );
        return;
      }

      // Cria a inscrição (salva o userId do usuário que criou o tracking)
      await Subscription.create({
        channelId: interaction.channelId,
        walletAddress: address,
        userId: interaction.user.id
      });

      logger.info(`✅ Inscrição criada: Canal ${interaction.channelId} → ${address.slice(0, 8)}`);

      await interaction.editReply(
        `✅ **Rastreamento Ativado!**\n\n` +
          `📡 **Carteira:** [\`${address.slice(0, 6)}...${address.slice(
            -4
          )}\`](https://polymarket.com/profile/${address})\n` +
          `⏰ Você receberá alertas de **mudanças no portfolio** (novas posições, aumentos, vendas).\n\n` +
          `💡 **Como funciona:** O bot compara o portfolio a cada 30s e detecta:\n` +
          `  • 🆕 Novas posições abertas\n` +
          `  • 📈 Aumentos em posições existentes\n` +
          `  • 📉 Reduções/vendas parciais\n` +
          `  • 🔴 Fechamento de posições\n\n`
      );
    } catch (error: any) {
      console.error("❌ Erro ao criar tracking:", error);
      await interaction.editReply(`❌ Erro interno ao salvar no banco de dados.\n` + `Detalhes: ${error.message}`);
    }
  }

  // ===== COMANDO /UNTRACK =====
  if (interaction.commandName === "untrack") {
    await interaction.deferReply();

    if (!rawInput) {
      await interaction.editReply("❌ Você precisa fornecer o endereço ou @username para desrastrear.");
      return;
    }

    const address = await resolveUser(rawInput);

    if (!address) {
      await interaction.editReply(`❌ Não encontrei essa carteira. Use o mesmo formato usado no \`/track\`.`);
      return;
    }

    try {
      // Remove a inscrição DESTE canal
      const deletedSub = await Subscription.findOneAndDelete({
        channelId: interaction.channelId,
        walletAddress: address
      });

      if (!deletedSub) {
        await interaction.editReply(`⚠️ Este canal não estava rastreando:\n` + `\`${address}\``);
        return;
      }

      // Garbage Collection: Remove carteira se não tem mais inscritos
      const remainingSubs = await Subscription.countDocuments({
        walletAddress: address
      });

      if (remainingSubs === 0) {
        await Wallet.findOneAndDelete({ address });
        console.log(`🗑️ Carteira ${address.slice(0, 8)} removida (0 inscritos)`);
      }

      await interaction.editReply(
        `✅ **Rastreamento Removido!**\n\n` +
          `Este canal não receberá mais alertas de:\n` +
          `\`${address.slice(0, 6)}...${address.slice(-4)}\``
      );
    } catch (error: any) {
      console.error("❌ Erro ao remover:", error);
      await interaction.editReply(`❌ Erro: ${error.message}`);
    }
  }

  // // ===== COMANDO /DEBUG (TESTE DE APIS) =====
  // if (interaction.commandName === "debug") {
  //   await interaction.deferReply();

  //   if (!rawInput) {
  //     await interaction.editReply(
  //       "❌ Você precisa fornecer um endereço para testar."
  //     );
  //     return;
  //   }

  //   const address = await resolveUser(rawInput);

  //   if (!address) {
  //     await interaction.editReply("❌ Endereço inválido.");
  //     return;
  //   }

  //   await interaction.editReply(
  //     `🧪 **Testando APIs da Polymarket...**\n\n` +
  //       `Endereço: \`${address}\`\n\n` +
  //       `Aguarde, isso pode levar alguns segundos...`
  //   );

  //   // Executa teste no console
  //   await testAPIConnection(address);

  //   // Busca uma posição de exemplo para debug
  //   try {
  //     // fetchPortfolio descontinuado
  //   } catch (e: any) {
  //     await interaction.editReply(
  //       `⚠️ **Teste parcial concluído**\n\n` +
  //         `Erro: ${e.message}\n\n` +
  //         `Verifique o console para mais detalhes.`
  //     );
  //   }
  // }

  // ===== COMANDO /LIST =====
  if (interaction.commandName === "list") {
    await interaction.deferReply();

    try {
      const subs = await Subscription.find({
        channelId: interaction.channelId
      });

      if (subs.length === 0) {
        await interaction.editReply(
          `ℹ️ **Nenhuma carteira rastreada neste canal.**\n\n` + `Use \`/track <endereço>\` para começar a rastrear.`
        );
        return;
      }

      const embed = new EmbedBuilder()
        .setTitle(`📋 Carteiras Rastreadas`)
        .setDescription(`Este canal está rastreando **${subs.length}** carteira(s):`)
        .setColor(0x5865f2);

      for (const sub of subs) {
        const wallet = await Wallet.findOne({ address: sub.walletAddress });
        const username = wallet ? await getUsernameFromAddress(wallet.address) : null;
        const displayName = username ?? null;
        let description = "";
        if (displayName) {
          description = `User: ${displayName}\n`;
        }
        description += `Carteira: ${sub.walletAddress}`;
        const lastCheck = wallet?.lastTimestamp ? new Date(wallet.lastTimestamp).toLocaleString("pt-BR") : "Nunca";

        // Mostra quem criou o tracking
        let value = `[Ver perfil](https://polymarket.com/profile/${sub.walletAddress}) • Última checagem: ${lastCheck}`;
        if (sub.userId) {
          value += `\n👤 Tracking criado por: <@${sub.userId}>`;
        }

        embed.addFields({
          name: `${description}`,
          value: value,
          inline: false
        });
      }

      await interaction.editReply({ embeds: [embed] });
    } catch (error: any) {
      console.error("Erro ao listar:", error);
      await interaction.editReply(`❌ Erro: ${error.message}`);
    }
  }

  // ===== COMANDO /FILTER =====
  if (interaction.commandName === "filter") {
    await interaction.deferReply();

    if (!rawInput) {
      await interaction.editReply("❌ Você precisa fornecer um endereço ou @username.");
      return;
    }

    const address = await resolveUser(rawInput);
    if (!address) {
      await interaction.editReply(`❌ Carteira não encontrada para **${rawInput}**`);
      return;
    }

    const sub = await Subscription.findOne({
      channelId: interaction.channelId,
      walletAddress: address
    });

    if (!sub) {
      await interaction.editReply(
        `⚠️ Este canal não está rastreando a carteira: ${rawInput}\n` + `Use \`/track\` primeiro.`
      );
      return;
    }

    const keyword = interaction.options.getString("keyword");
    const minUsd = interaction.options.getNumber("min_usd");
    const clear = interaction.options.getBoolean("limpar");

    if (clear) {
      sub.filters = { keywords: [], minUsd: 0 };
      await sub.save();
      await interaction.editReply(`✅ Filtros removidos para \`${address.slice(0, 8)}\`!`);
      return;
    }

    // Inicializa se não existir
    if (!sub.filters) sub.filters = { keywords: [], minUsd: 0 };
    if (!sub.filters.keywords) sub.filters.keywords = [];

    const updates: string[] = [];

    if (minUsd !== null) {
      sub.filters.minUsd = minUsd;
      updates.push(`• Mínimo USD: **$${minUsd}**`);
    }

    if (keyword) {
      // Se já existe, não adiciona duplicado
      if (!sub.filters.keywords.includes(keyword)) {
        sub.filters.keywords.push(keyword);
        updates.push(`• Keyword adicionada: **"${keyword}"**`);
      } else {
        updates.push(`• Keyword já existe: "${keyword}"`);
      }
    }

    if (updates.length === 0) {
      await interaction.editReply(
        `ℹ️ Nenhum filtro alterado.\n\n` +
          `Filtros atuais:\n` +
          `• Min USD: $${sub.filters.minUsd || 0}\n` +
          `• Keywords: ${sub.filters.keywords.join(", ") || "(nenhuma)"}`
      );
      return;
    }

    await sub.save();

    await interaction.editReply(
      `✅ **Filtros Atualizados!**\n` +
        `Carteira: \`${address.slice(0, 8)}\`\n\n` +
        updates.join("\n") +
        `\n\nFiltros ativos:\n` +
        `• Min USD: $${sub.filters.minUsd || 0}\n` +
        `• Keywords: ${sub.filters.keywords.join(", ") || "(nenhuma)"}`
    );
  }

  // ===== COMANDO /HELP =====
  if (interaction.commandName === "help") {
    await interaction.deferReply();

    const embed = new EmbedBuilder()
      .setTitle("🤖 Polymarket Tracker - Ajuda")
      .setDescription(
        `Bot para rastrear apostas em tempo real na Polymarket.\n\n` +
          `**Como funciona:**\n` +
          `O bot monitora carteiras e envia alertas quando novas apostas são feitas.`
      )
      .setColor(0x5865f2)
      .addFields(
        {
          name: "📡 `/track <endereço>`",
          value:
            "Começa a rastrear uma carteira. Você receberá alertas de novas apostas.\n" +
            "Aceita: `0x123...abc` ou `@username`",
          inline: false
        },
        {
          name: "🚫 `/untrack <endereço>`",
          value: "Para de rastrear uma carteira neste canal.",
          inline: false
        },
        {
          name: "📋 `/list`",
          value: "Lista todas as carteiras rastreadas neste canal.",
          inline: false
        },
        {
          name: "🔍 `/filter`",
          value:
            "Configura filtros (ex: palavra-chave, valor mínimo).\nUso: `/filter carteira:@user keyword:Trump min_usd:100`",
          inline: false
        }
        // {
        //   name: "🧪 `/debug <endereço>`",
        //   value:
        //     "Testa a conectividade com as APIs (útil se algo não estiver funcionando).",
        //   inline: false
        // }
      )
      .setFooter({
        text: "💡 Dica: Use @username para facilitar (ex: @GCR)"
      });

    await interaction.editReply({ embeds: [embed] });
  }
});
