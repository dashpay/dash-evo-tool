# Prints the value of the variable `name` from an env file, with no trailing
# newline: without an `export ` prefix, CR line ending, surrounding quotes or
# trailing `# comment`. Prints nothing and exits 1 when the variable is
# missing, empty, or written in any other form (blanks around `=`, an unclosed
# quote, text after the closing quote, escapes), so a caller never acts on a
# half-read secret.
# usage: awk -v name=VAR -f env-value.awk FILE
{
  sub(/\r$/, "")
  line = $0
  sub(/^export[ \t]+/, "", line)
  prefix = name "="
  if (substr(line, 1, length(prefix)) != prefix) next
  value = substr(line, length(prefix) + 1)
  if (value ~ /^[ \t]/) exit 1
  quote = substr(value, 1, 1)
  if (quote == "\"" || quote == "'") {
    rest = substr(value, 2)
    close_at = index(rest, quote)
    if (close_at == 0) exit 1
    if (substr(rest, close_at + 1) !~ /^[ \t]*(#.*)?$/) exit 1
    value = substr(rest, 1, close_at - 1)
    if (index(value, "\\") > 0) exit 1
  } else {
    sub(/[ \t]+#.*$/, "", value)
    sub(/[ \t]+$/, "", value)
  }
  if (value == "") exit 1
  printf "%s", value
  found = 1
  exit 0
}
END { if (!found) exit 1 }
