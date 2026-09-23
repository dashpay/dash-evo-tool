# SemVer precedence ignores build metadata and orders final releases last.
def semver_key:
    . as $tag |
    capture("^v?(?<major>0|[1-9][0-9]*)\\.(?<minor>0|[1-9][0-9]*)\\.(?<patch>0|[1-9][0-9]*)(?:-(?<pre>[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*))?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$") // error("Invalid release tag: \($tag)") |
    [(.major | tonumber), (.minor | tonumber), (.patch | tonumber),
     (if .pre == null then 1 else 0 end),
     ((.pre // "") | split(".") | map(
         if test("^[0-9]+$") then
             if test("^0[0-9]") then error("Invalid prerelease: \($tag)") else tonumber end
         else . end))];
