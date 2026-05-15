export enum LogLevel {
  DEBUG = 0,
  INFO = 1,
  WARN = 2,
  ERROR = 3,
  NONE = 4
}

const getLogLevel = (): LogLevel => {
  const envLevel = (process.env.LOG_LEVEL || "INFO").toUpperCase();
  // @ts-ignore
  return LogLevel[envLevel] !== undefined ? LogLevel[envLevel] : LogLevel.INFO;
};

const currentLevel = getLogLevel();

export const logger = {
  debug: (message: string, ...args: any[]) => {
    if (currentLevel <= LogLevel.DEBUG) {
      console.log(`[DEBUG] ${message}`, ...args);
    }
  },
  info: (message: string, ...args: any[]) => {
    if (currentLevel <= LogLevel.INFO) {
      console.log(`[INFO] ${message}`, ...args);
    }
  },
  warn: (message: string, ...args: any[]) => {
    if (currentLevel <= LogLevel.WARN) {
      console.warn(`[WARN] ${message}`, ...args);
    }
  },
  error: (message: string, ...args: any[]) => {
    if (currentLevel <= LogLevel.ERROR) {
      console.error(`[ERROR] ${message}`, ...args);
    }
  }
};
