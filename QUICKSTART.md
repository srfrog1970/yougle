# Quickstart

Get Yougle running on your phone and, if you want durable delivery instead
of just live device-to-device messaging, connect it to your own
self-hosted mailbox. Android only — iOS requires Xcode/macOS to build,
which hasn't been attempted anywhere in this project (see
[`app/README.md`](app/README.md)'s Environment note).

## 1. Install the app

Yougle isn't on the Play Store — no accounts, no shared infrastructure
means no app-store account either. Instead, download the signed APK
directly from this repo's [Releases page](../../releases) and sideload
it:

1. On your Android phone, open the latest release and download
   `app-release.apk`.
2. Tap the downloaded file. Android will block the install the first
   time and prompt you to allow installs from that source (browser or
   file manager) — this is expected for any app installed outside the
   Play Store, not a Yougle-specific warning. Allow it, then install.
3. Open **Yougle**.

The APK is signed with a dedicated Yougle release key (not a debug
key) and is a normal release build — not debuggable, no bundled dev
server dependency.

## 2. First launch

On first launch the app silently generates a new identity (a 24-word
recovery phrase, held only on your device) and lands on an empty
conversation list. Tap **View phrase** on the banner and write the
24 words down somewhere safe — it's the only way to recover your
identity if you lose this device. There's no server-side account
recovery, by design.

Every identity starts **Local-only**: you can pair with contacts and
exchange messages live, but delivery only works while both phones are
online and reachable at the same time. Section 4 below covers adding a
Server mailbox for durable, asynchronous delivery.

## 3. Pair with a contact

Tap **+ Pair**. Two ways to exchange codes, both equivalent:

- **QR code**: one person shows their **My code** QR, the other scans
  it from their own **+ Pair → Enter code** screen (camera scanning) or
  by pasting the long `yougle-pair-v1:...` text underneath the code.
- **Paste code**: copy/paste the same text through any channel you
  already trust (in person, an existing secure chat, etc.) if scanning
  isn't convenient.

Pairing is mutual — both sides need to add each other for messages to
flow. Once paired, the contact appears on the conversation list.

## 4. Send a message

Open the contact from the conversation list and type a message. If
both devices are Local-only and both happen to be online, delivery is
live and you'll see the status move from **Sent** to **Delivered**. If
either side is offline, the message queues and delivers automatically
once both are reachable — no manual retry needed.

## 5. Want durable delivery instead of just Local?

By default, a message to an offline contact just waits for both of you
to be online at the same time. Running your own **Server mailbox**
(`pm-node`) removes that requirement: messages addressed to you land on
your node and wait there durably until you next open the app, even if
your phone was off or unreachable the whole time. It's a single-tenant
node you run yourself — never a shared community server, and never
required to use the app.

Setting one up (a Raspberry Pi, a spare machine, a small VPS — anything
that can stay on) is covered start to finish in
[`deploy/README.md`](deploy/README.md), including where in the app to
find the two key values (`Manage Mailbox → Set up your own node`) your
node needs to identify itself as yours, and where in the app to point
your phone at the node's printed address once it's running.

## Notes

- Sending, receiving, pairing, and Server-mailbox setup are all
  Android-verified end to end (see [`README.md`](README.md)'s status
  line). This APK itself was smoke-tested on-device after signing:
  installed with no Metro/dev-server running at all, launched cleanly,
  and walked through onboarding, identity creation, and the pairing
  screen.
- Building from source (for development, or to produce your own signed
  build) is covered in [`app/README.md`](app/README.md).
