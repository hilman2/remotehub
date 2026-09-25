/**
 * KeePass' 69 standard icons (#98), by their number, drawn with the closest
 * Lucide icon. Entries keep the number, so a KeePass file gets back what it
 * brought (#99).
 */
import AppWindow from '@lucide/svelte/icons/app-window';
import Apple from '@lucide/svelte/icons/apple';
import Archive from '@lucide/svelte/icons/archive';
import Banknote from '@lucide/svelte/icons/banknote';
import BatteryWarning from '@lucide/svelte/icons/battery-warning';
import Book from '@lucide/svelte/icons/book';
import BookOpen from '@lucide/svelte/icons/book-open';
import Camera from '@lucide/svelte/icons/camera';
import CircleCheck from '@lucide/svelte/icons/circle-check';
import CircleX from '@lucide/svelte/icons/circle-x';
import ClipboardCheck from '@lucide/svelte/icons/clipboard-check';
import Clock from '@lucide/svelte/icons/clock';
import Cpu from '@lucide/svelte/icons/cpu';
import Disc from '@lucide/svelte/icons/disc';
import Feather from '@lucide/svelte/icons/feather';
import FileCheck from '@lucide/svelte/icons/file-check';
import FileLock from '@lucide/svelte/icons/file-lock';
import FilePlus from '@lucide/svelte/icons/file-plus';
import FileQuestionMark from '@lucide/svelte/icons/file-question-mark';
import Flag from '@lucide/svelte/icons/flag';
import Folder from '@lucide/svelte/icons/folder';
import FolderArchive from '@lucide/svelte/icons/folder-archive';
import FolderCheck from '@lucide/svelte/icons/folder-check';
import FolderOpen from '@lucide/svelte/icons/folder-open';
import Globe from '@lucide/svelte/icons/globe';
import GlobeLock from '@lucide/svelte/icons/globe-lock';
import Hammer from '@lucide/svelte/icons/hammer';
import HardDrive from '@lucide/svelte/icons/hard-drive';
import House from '@lucide/svelte/icons/house';
import IdCard from '@lucide/svelte/icons/id-card';
import Image from '@lucide/svelte/icons/image';
import Inbox from '@lucide/svelte/icons/inbox';
import Info from '@lucide/svelte/icons/info';
import KeyRound from '@lucide/svelte/icons/key-round';
import KeySquare from '@lucide/svelte/icons/key-square';
import Landmark from '@lucide/svelte/icons/landmark';
import LayoutGrid from '@lucide/svelte/icons/layout-grid';
import List from '@lucide/svelte/icons/list';
import LockOpen from '@lucide/svelte/icons/lock-open';
import Mail from '@lucide/svelte/icons/mail';
import MailSearch from '@lucide/svelte/icons/mail-search';
import MemoryStick from '@lucide/svelte/icons/memory-stick';
import MessagesSquare from '@lucide/svelte/icons/messages-square';
import Monitor from '@lucide/svelte/icons/monitor';
import MonitorSmartphone from '@lucide/svelte/icons/monitor-smartphone';
import Notebook from '@lucide/svelte/icons/notebook';
import Package from '@lucide/svelte/icons/package';
import Pen from '@lucide/svelte/icons/pen';
import Play from '@lucide/svelte/icons/play';
import Printer from '@lucide/svelte/icons/printer';
import Puzzle from '@lucide/svelte/icons/puzzle';
import Radio from '@lucide/svelte/icons/radio';
import Save from '@lucide/svelte/icons/save';
import Scan from '@lucide/svelte/icons/scan';
import ScrollText from '@lucide/svelte/icons/scroll-text';
import Server from '@lucide/svelte/icons/server';
import Settings from '@lucide/svelte/icons/settings';
import Smartphone from '@lucide/svelte/icons/smartphone';
import Sparkles from '@lucide/svelte/icons/sparkles';
import SquareTerminal from '@lucide/svelte/icons/square-terminal';
import Star from '@lucide/svelte/icons/star';
import StickyNote from '@lucide/svelte/icons/sticky-note';
import Terminal from '@lucide/svelte/icons/terminal';
import Trash2 from '@lucide/svelte/icons/trash-2';
import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
import Tv from '@lucide/svelte/icons/tv';
import UserLock from '@lucide/svelte/icons/user-lock';
import Wrench from '@lucide/svelte/icons/wrench';
import Zap from '@lucide/svelte/icons/zap';

/** Position = KeePass' icon number (0: the key, 48: the folder). */
export const ICONS: (typeof KeyRound)[] = [
	KeyRound,
	Globe,
	TriangleAlert,
	Server,
	FolderCheck,
	MessagesSquare,
	Puzzle,
	Notebook,
	GlobeLock,
	IdCard,
	FileCheck,
	Camera,
	Radio,
	KeySquare,
	Zap,
	Scan,
	Sparkles,
	Disc,
	Monitor,
	Mail,
	Settings,
	ClipboardCheck,
	FilePlus,
	Tv,
	BatteryWarning,
	Inbox,
	Save,
	HardDrive,
	FileQuestionMark,
	SquareTerminal,
	Terminal,
	Printer,
	LayoutGrid,
	Play,
	Wrench,
	MonitorSmartphone,
	Archive,
	Landmark,
	AppWindow,
	Clock,
	MailSearch,
	Flag,
	MemoryStick,
	Trash2,
	StickyNote,
	CircleX,
	Info,
	Package,
	Folder,
	FolderOpen,
	FolderArchive,
	LockOpen,
	FileLock,
	CircleCheck,
	Pen,
	Image,
	Book,
	List,
	UserLock,
	Hammer,
	House,
	Star,
	Cpu,
	Feather,
	Apple,
	BookOpen,
	Banknote,
	ScrollText,
	Smartphone
];

/** KeePass' icon for a new group. */
export const FOLDER_ICON = 48;

/** The icon of a number, the key for one out of range. */
export const icon = (number: number | undefined) => ICONS[number ?? 0] ?? ICONS[0];
