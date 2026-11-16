# Security policy

## Reporting a vulnerability

Please report it **privately**, not in a public issue.

Go to the **Security** tab of this repository, choose **Report a vulnerability**, and fill in the form. Only you and the maintainer can see the report.

Please include:

- what happens and why you think it is a security problem,
- the steps to reproduce it (a small example document or command helps),
- the version (`researcher --version`), the operating system, and whether you used the Rust binary or `researcher.py`.

You will get an acknowledgement as soon as I see the report. For a verified problem I aim to have a fix released within about 90 days, and I will tell you if it will take longer. If you want to be credited in the release notes, say so.

## Supported versions

Only the latest release gets security fixes. Please check that the problem is still there in the latest release before you report it.

## What is in scope

- the `researcher` binary and `researcher.py`: reading files from the library, converting them to text, the text copies and the cache, the first-use popup;
- the release workflow and the files attached to a release (the binary and `SHA256SUMS.txt`);
- the usage Worker in `worker/` and the opt-in sending of anonymous usage counts.

## What is not a vulnerability by itself

Documents are loaded into Claude's context as text, and a saved web page or PDF can contain instructions aimed at an AI. The skill tells Claude to treat document contents as reference material, not as commands, and the README says to load only documents you trust (see "Safety" there). A document that contains such instructions is a known limit, not a vulnerability. A way to make the loader hide, add or change text, read files outside the library, or send data that the README says is not sent **is** in scope.

## Disclosure

Please give me the time above before you publish details. When a fix is released I will publish a GitHub security advisory that names the affected versions.
