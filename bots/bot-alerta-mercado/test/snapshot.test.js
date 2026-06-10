import test from 'node:test';
import assert from 'node:assert/strict';
import { buildSnapshot, diffSnapshots } from '../src/snapshot.js';

function eventWith(markets) {
  return {
    id: '45915',
    slug: 'brazil-presidential-election',
    title: 'Brazil Presidential Election',
    updatedAt: '2026-05-20T11:00:00Z',
    markets,
  };
}

test('detecta candidato novo criado como mercado novo', () => {
  const previous = buildSnapshot(
    eventWith([
      {
        id: '1',
        slug: 'will-a-win',
        question: 'Will A win?',
        groupItemTitle: 'A',
        active: true,
        acceptingOrders: true,
      },
    ]),
  );
  const current = buildSnapshot(
    eventWith([
      {
        id: '1',
        slug: 'will-a-win',
        question: 'Will A win?',
        groupItemTitle: 'A',
        active: true,
        acceptingOrders: true,
      },
      {
        id: '2',
        slug: 'will-b-win',
        question: 'Will B win?',
        groupItemTitle: 'B',
        active: true,
        acceptingOrders: true,
      },
    ]),
  );

  const changes = diffSnapshots(previous, current);
  assert.equal(changes.length, 1);
  assert.equal(changes[0].type, 'candidate_added');
  assert.equal(changes[0].market.candidate, 'B');
});

test('detecta placeholder virando candidato real sem alertar ativacao separada', () => {
  const previous = buildSnapshot(
    eventWith([
      {
        id: '10',
        slug: 'will-person-x-win-the-2026-brazilian-presidential-election',
        question: 'Will Person X win the 2026 Brazilian presidential election?',
        groupItemTitle: 'Person X',
        active: false,
        acceptingOrders: true,
      },
    ]),
  );
  const current = buildSnapshot(
    eventWith([
      {
        id: '10',
        slug: 'will-renan-santos-win-the-2026-brazilian-presidential-election',
        question: 'Will Renan Santos win the 2026 Brazilian presidential election?',
        groupItemTitle: 'Renan Santos',
        active: true,
        acceptingOrders: true,
      },
    ]),
  );

  const changes = diffSnapshots(previous, current);
  assert.deepEqual(changes.map((change) => change.type), ['candidate_added']);
});

test('ignora mudancas comuns de preco e volume', () => {
  const previous = buildSnapshot(
    eventWith([
      {
        id: '1',
        slug: 'will-a-win',
        question: 'Will A win?',
        groupItemTitle: 'A',
        active: true,
        acceptingOrders: true,
        bestBid: 0.1,
        bestAsk: 0.11,
        volume: '100',
      },
    ]),
  );
  const current = buildSnapshot(
    eventWith([
      {
        id: '1',
        slug: 'will-a-win',
        question: 'Will A win?',
        groupItemTitle: 'A',
        active: true,
        acceptingOrders: true,
        bestBid: 0.12,
        bestAsk: 0.13,
        volume: '120',
      },
    ]),
  );

  assert.equal(diffSnapshots(previous, current).length, 0);
});

test('ignora mercado novo que ainda e placeholder', () => {
  const previous = buildSnapshot(eventWith([]));
  const current = buildSnapshot(
    eventWith([
      {
        id: '99',
        slug: 'will-person-z-win-the-2026-brazilian-presidential-election',
        question: 'Will Person Z win the 2026 Brazilian presidential election?',
        groupItemTitle: 'Person Z',
        active: false,
        acceptingOrders: true,
      },
    ]),
  );

  assert.equal(diffSnapshots(previous, current).length, 0);
});

test('ignora mercado novo de Other porque nao e candidato real', () => {
  const previous = buildSnapshot(eventWith([]));
  const current = buildSnapshot(
    eventWith([
      {
        id: '100',
        slug: 'will-another-person-win-the-2026-brazilian-presidential-election',
        question: 'Will another person win the 2026 Brazilian presidential election?',
        groupItemTitle: 'Other',
        active: false,
        acceptingOrders: true,
      },
    ]),
  );

  assert.equal(diffSnapshots(previous, current).length, 0);
});

test('ignora mudanca de status sem candidato novo', () => {
  const previous = buildSnapshot(
    eventWith([
      {
        id: '1',
        slug: 'will-a-win',
        question: 'Will A win?',
        groupItemTitle: 'A',
        active: false,
        acceptingOrders: false,
      },
    ]),
  );
  const current = buildSnapshot(
    eventWith([
      {
        id: '1',
        slug: 'will-a-win',
        question: 'Will A win?',
        groupItemTitle: 'A',
        active: true,
        acceptingOrders: true,
      },
    ]),
  );

  assert.equal(diffSnapshots(previous, current).length, 0);
});
