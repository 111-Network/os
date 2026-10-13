// Canned agent replies for the mock. Mirrors the real agent's shape: the streamed deltas carry the
// whole turn (lead-in, a tool call, then the answer), while `done.reply` is only the final
// sentence. Each reply streams as 25-60 word tokens.
export interface CannedReply {
  /** Streamed before the tool call. */
  pre: string;
  /** Streamed after the tool call; its last sentence becomes `done.reply`. */
  post: string;
}

const REPLIES: { match: RegExp; reply: CannedReply }[] = [
  {
    match: /radio|lora|meshtastic|antenna/i,
    reply: {
      pre: "Let me check the knowledge base for the LoRa radio recipe. ",
      post: "Meshtastic needs a supported LoRa board on USB and the meshtasticd service running here. Plug the board in and wait a few seconds, then ask me to test it. If no device shows up, try a different cable, since many only charge.",
    },
  },
  {
    match: /model|llama|slow|memory|context/i,
    reply: {
      pre: "I will look at the model server notes first. ",
      post: "The local model runs on this node with a four thousand token window, which is enough for short troubleshooting chats. If longer conversations get cut off, I can write a proposal to raise the window, and you decide whether to accept it.",
    },
  },
  {
    match: /mesh|transport|reticulum|peer|neighbou?r|nearby/i,
    reply: {
      pre: "Checking the Reticulum page and the transport health now. ",
      post: "The mesh transport is running and it has heard another node recently, so messages can leave this device. Nothing needs fixing right now. If the connection drops again, ask me to diagnose it and I will leave a proposal you can review.",
    },
  },
  {
    match: /.*/s,
    reply: {
      pre: "Let me check the knowledge base first so I give you the right steps. ",
      post: "Your node is running and every core service is healthy. I can explain any service in more detail, or look at its log and write a proposal for you to review. Just tell me which part you want to understand or fix.",
    },
  },
];

export function pickReply(message: string): CannedReply {
  return (REPLIES.find((r) => r.match.test(message)) ?? REPLIES[REPLIES.length - 1]!).reply;
}

/** Split text into word tokens that keep their trailing whitespace. */
export function tokenize(text: string): string[] {
  return text.match(/\S+\s*/g) ?? [];
}

/** The last sentence of a passage. */
export function finalSentence(text: string): string {
  const sentences = text.trim().match(/[^.!?]+[.!?]+/g);
  return (sentences ? sentences[sentences.length - 1]! : text).trim();
}
