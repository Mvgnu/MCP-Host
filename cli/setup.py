from pathlib import Path
from setuptools import find_packages, setup

root = Path(__file__).resolve().parents[1]
readme = (root / "README.md").read_text()

setup(
    name="mcp-host-cli",
    version="0.2.0",
    description="Mission-control CLI utilities for MCP Host",
    long_description=readme,
    long_description_content_type="text/markdown",
    packages=find_packages(where="cli", include=["mcpctl", "mcpctl.*"]),
    package_dir={"": "cli"},
    entry_points={"console_scripts": ["mcpctl=mcpctl.cli:main"]},
    install_requires=["requests"],
    extras_require={"scaffold": ["fastapi", "uvicorn"]},
    python_requires=">=3.9",
)
