-- Tabela principal de personagens do Aurenfall
CREATE TABLE IF NOT EXISTS characters (
    character_id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL,
    name VARCHAR(64) NOT NULL UNIQUE,
    level INT NOT NULL DEFAULT 1,
    experience BIGINT NOT NULL DEFAULT 0,
    available_attribute_points INT NOT NULL DEFAULT 15,
    available_skill_points INT NOT NULL DEFAULT 0,
    
    -- Atributos Básicos (Item 1.2 da nossa design doc)
    strength INT NOT NULL DEFAULT 0,
    defense INT NOT NULL DEFAULT 0,
    agility INT NOT NULL DEFAULT 0,
    vitality INT NOT NULL DEFAULT 0,
    intelligence INT NOT NULL DEFAULT 0,
    
    -- Posição atual no mundo infinito (Coordenadas em milímetros i64)
    pos_x_mm BIGINT NOT NULL DEFAULT 0,
    pos_y_mm BIGINT NOT NULL DEFAULT 0,
    pos_z_mm BIGINT NOT NULL DEFAULT 0,
    current_quadrant_x BIGINT NOT NULL DEFAULT 0,
    current_quadrant_y BIGINT NOT NULL DEFAULT 0,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Tabela para armazenar as Qualidades e Defeitos escolhidos na criação (Item 1.1)
CREATE TABLE IF NOT EXISTS character_traits (
    id BIGSERIAL PRIMARY KEY,
    character_id BIGINT NOT NULL REFERENCES characters(character_id) ON DELETE CASCADE,
    trait_type VARCHAR(16) NOT NULL, -- 'Quality' ou 'Flaw'
    trait_name VARCHAR(64) NOT NULL,
    UNIQUE(character_id, trait_name)
);