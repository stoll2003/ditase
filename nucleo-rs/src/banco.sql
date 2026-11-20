CREATE TABLE IF NOT EXISTS ditados (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    iniciado_em REAL NOT NULL, duracao REAL NOT NULL DEFAULT 0, duracao_fala REAL NOT NULL DEFAULT 0,
    palavras INTEGER NOT NULL DEFAULT 0, texto_bruto TEXT NOT NULL DEFAULT '', texto TEXT NOT NULL DEFAULT '',
    app TEXT NOT NULL DEFAULT '', app_nome TEXT NOT NULL DEFAULT '', estado TEXT NOT NULL DEFAULT 'gravando',
    latencia REAL NOT NULL DEFAULT 0, motor TEXT NOT NULL DEFAULT '', limpador TEXT NOT NULL DEFAULT '',
    trocas_dicionario INTEGER NOT NULL DEFAULT 0, origem TEXT NOT NULL DEFAULT 'ditado', audio TEXT NOT NULL DEFAULT '',
    chave_externa TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS ditados_por_data ON ditados (iniciado_em);
CREATE TABLE IF NOT EXISTS trechos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ditado_id INTEGER NOT NULL REFERENCES ditados(id) ON DELETE CASCADE,
    ordem INTEGER NOT NULL, bruto TEXT NOT NULL, limpo TEXT
);
CREATE INDEX IF NOT EXISTS trechos_por_ditado ON trechos (ditado_id, ordem);
CREATE TABLE IF NOT EXISTS dicionario (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    termo TEXT NOT NULL UNIQUE COLLATE NOCASE, ouvido_como TEXT NOT NULL DEFAULT '[]',
    usos INTEGER NOT NULL DEFAULT 0, origem TEXT NOT NULL DEFAULT 'manual', criado_em REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS transcricoes_longas (
    id TEXT PRIMARY KEY, tipo TEXT NOT NULL, criado_em REAL NOT NULL, dados TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS transcricoes_longas_por_tipo ON transcricoes_longas (tipo, criado_em DESC);
CREATE TABLE IF NOT EXISTS config (chave TEXT PRIMARY KEY, valor TEXT NOT NULL);
