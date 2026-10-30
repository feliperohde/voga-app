-- Built-in models (llama.cpp on this computer) are the default; Ollama, the network and
-- cloud providers are options.

-- builtin | ollama
ALTER TABLE ai_settings ADD COLUMN embedding_provider TEXT NOT NULL DEFAULT 'builtin';
-- An index already embedded by Ollama keeps its model
UPDATE ai_settings SET embedding_provider = 'ollama'
WHERE EXISTS (SELECT 1 FROM rag_chunks WHERE embedding IS NOT NULL AND embedding_model NOT LIKE 'builtin:%');
UPDATE ai_settings SET embedding_model = 'bge-m3-q8' WHERE embedding_provider = 'builtin';
-- No chat model chosen yet: the built-in one
UPDATE ai_settings SET provider = 'builtin-ai' WHERE model = '';
