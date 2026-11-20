CREATE TABLE IF NOT EXISTS lt_documentos (
    id TEXT PRIMARY KEY, tipo TEXT NOT NULL, nome TEXT NOT NULL, criado_em REAL NOT NULL,
    fonte TEXT, estado TEXT NOT NULL, duracao REAL NOT NULL DEFAULT 0, oculto INTEGER NOT NULL DEFAULT 0,
    versao_ativa TEXT, falantes TEXT NOT NULL DEFAULT 'pendente',
    resumo TEXT, resumo_gerado INTEGER NOT NULL DEFAULT 0, erro_resumo TEXT, resumo_cancelado INTEGER NOT NULL DEFAULT 0,
    erro_captura TEXT, aviso_captura TEXT, aviso TEXT, atraso REAL
);
CREATE INDEX IF NOT EXISTS lt_documentos_tipo ON lt_documentos (tipo, criado_em DESC);
CREATE TABLE IF NOT EXISTS lt_blocos (
    id TEXT PRIMARY KEY, documento TEXT NOT NULL, fonte TEXT NOT NULL, geracao INTEGER NOT NULL, seq INTEGER NOT NULL,
    inicio INTEGER NOT NULL, quantidade INTEGER NOT NULL, arquivo TEXT, tamanho INTEGER, soma TEXT,
    estado TEXT NOT NULL, tentativas INTEGER NOT NULL DEFAULT 0, erro TEXT, audio_liberado INTEGER NOT NULL DEFAULT 0,
    UNIQUE (documento, fonte, geracao, seq)
);
CREATE INDEX IF NOT EXISTS lt_blocos_doc ON lt_blocos (documento, fonte, geracao, inicio);
CREATE INDEX IF NOT EXISTS lt_blocos_fim ON lt_blocos (documento, fonte, geracao, inicio + quantidade);
CREATE INDEX IF NOT EXISTS lt_blocos_capturados_fim ON lt_blocos (documento, fonte, geracao, inicio + quantidade) WHERE estado='capturado';
CREATE TABLE IF NOT EXISTS lt_costuras (
    documento TEXT NOT NULL, fonte TEXT NOT NULL, geracao INTEGER NOT NULL,
    confirmado_ate INTEGER NOT NULL, dono TEXT, fechada INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (documento, fonte, geracao)
);
CREATE TABLE IF NOT EXISTS lt_pendencias (
    id TEXT PRIMARY KEY, documento TEXT NOT NULL, fonte TEXT NOT NULL, geracao INTEGER NOT NULL,
    inicio INTEGER NOT NULL, fim INTEGER NOT NULL, motivo TEXT NOT NULL,
    tentativas INTEGER NOT NULL DEFAULT 1, estado TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS lt_pendencias_intervalo ON lt_pendencias (documento, fonte, geracao, estado, inicio);
CREATE TABLE IF NOT EXISTS lt_trechos (
    id TEXT PRIMARY KEY, documento TEXT NOT NULL, versao TEXT NOT NULL DEFAULT '', fonte TEXT NOT NULL,
    inicio REAL NOT NULL, fim REAL NOT NULL, texto TEXT NOT NULL, voz TEXT NOT NULL, voz_final TEXT,
    divergente INTEGER NOT NULL DEFAULT 0, origem_legado TEXT UNIQUE, criado_em REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS lt_trechos_doc ON lt_trechos (documento, versao, inicio);
CREATE INDEX IF NOT EXISTS lt_trechos_centro ON lt_trechos (documento, fonte, (inicio + fim) * 8000) WHERE versao='';
CREATE TABLE IF NOT EXISTS lt_edicoes (
    id INTEGER PRIMARY KEY AUTOINCREMENT, documento TEXT NOT NULL, versao TEXT NOT NULL DEFAULT '',
    tipo TEXT NOT NULL, alvo TEXT NOT NULL, valor TEXT NOT NULL, criado_em REAL NOT NULL, chave TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS lt_edicoes_doc ON lt_edicoes (documento, versao, id);
CREATE TABLE IF NOT EXISTS lt_versoes (
    id TEXT PRIMARY KEY, documento TEXT NOT NULL, tentativa INTEGER NOT NULL, origem TEXT NOT NULL,
    identidade TEXT, estado TEXT NOT NULL, criada_em REAL NOT NULL, publicada_em REAL,
    duracao REAL, bruto TEXT, processamento REAL, baixa_confianca INTEGER NOT NULL DEFAULT 0, erro TEXT,
    UNIQUE (documento, tentativa)
);
CREATE TABLE IF NOT EXISTS lt_janelas_arquivo (
    versao TEXT NOT NULL, seq INTEGER NOT NULL, inicio INTEGER NOT NULL, fim INTEGER NOT NULL,
    soma TEXT NOT NULL, estado TEXT NOT NULL, PRIMARY KEY (versao, seq)
);
CREATE TABLE IF NOT EXISTS lt_descartes (documento TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS lt_migracoes (chave TEXT PRIMARY KEY, feita_em REAL NOT NULL, detalhe TEXT);
