#!/usr/bin/env python3
import os

OUTPUT_FILE = "project_dump.txt"

# Directories to ignore entirely
EXCLUDE_DIRS = {".git", "target", "node_modules", "dist", "build", ".svelte-kit"}

# Specific files to ignore
EXCLUDE_FILES = {
    ".env",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    ".gitignore",
    OUTPUT_FILE,
    "dump_project.py"
}

# File extensions to ignore
EXCLUDE_EXTENSIONS = {".csv", ".7z", ".zip", ".rar", ".tar", ".gz", ".png", ".jpg", ".jpeg", ".ico", ".svg"}

def is_text_file(filepath):
    """Check if a file is text by attempting to read it."""
    try:
        with open(filepath, 'tr', encoding='utf-8') as check_file:
            check_file.read(1024)
            return True
    except UnicodeDecodeError:
        return False
    except Exception:
        return False

def main():
    root_dir = "."
    
    with open(OUTPUT_FILE, 'w', encoding='utf-8') as outfile:
        # Walk through the directory tree
        for current_root, dirs, files in os.walk(root_dir):
            # Modify dirs in-place to prevent os.walk from entering excluded directories
            dirs[:] = [d for d in dirs if d not in EXCLUDE_DIRS]
            
            for file in sorted(files):
                # Skip excluded files
                if file in EXCLUDE_FILES:
                    continue
                
                # Skip files with excluded extensions
                _, ext = os.path.splitext(file)
                if ext.lower() in EXCLUDE_EXTENSIONS:
                    continue
                
                filepath = os.path.join(current_root, file)
                
                # Further ensure we don't accidentally dump binary files
                if not is_text_file(filepath):
                    print(f"Skipping binary/non-text file: {filepath}")
                    continue
                
                try:
                    with open(filepath, 'r', encoding='utf-8') as infile:
                        content = infile.read()
                        
                    # Write separator and file path
                    outfile.write(f"{'='*64}\n")
                    outfile.write(f"File: {filepath}\n")
                    outfile.write(f"{'='*64}\n")
                    
                    # Write file content
                    outfile.write(content)
                    outfile.write("\n\n")
                    
                    print(f"Included: {filepath}")
                except Exception as e:
                    print(f"Error reading {filepath}: {e}")

    print(f"\n✅ Project successfully dumped to {OUTPUT_FILE}")

if __name__ == "__main__":
    main()
