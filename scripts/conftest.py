import sys
from pathlib import Path

# Add scripts directory to Python path so ci_agent can be imported
scripts_dir = Path(__file__).parent
sys.path.insert(0, str(scripts_dir))
