# test-repo

## Branch workflow

- Develop on `dev` or short-lived feature branches.
- Open release pull requests from `dev` to `main`.
- Pull requests are squash-merged.
- After merging into `main`, merge `main` back into `dev`.

Never commit credentials or other secrets.
