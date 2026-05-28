cargo build --release 
# binary at ./target/release/reemerge
# optionally:
Copy-Item ./target/release/reemerge.exe .
# Remove build artifacts (binary was copied above)
Remove-Item ./target -Recurse -Force

