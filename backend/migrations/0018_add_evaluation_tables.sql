-- Track evaluation tests and results for automated context evaluation
CREATE TABLE evaluation_tests (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    expected_answer TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT now()
);

CREATE TABLE evaluation_results (
    id SERIAL PRIMARY KEY,
    test_id INTEGER NOT NULL REFERENCES evaluation_tests(id) ON DELETE CASCADE,
    response TEXT NOT NULL,
    score DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT now()
);
