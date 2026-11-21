using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Reflection;
using System.Security.Cryptography;
using System.Web.Script.Serialization;

namespace JuleHygge
{
    // Self-update from GitHub releases. The Windows workflow builds JuleHygge.exe with the release tag as its
    // version and attaches it to the release. The app asks the GitHub API for the latest release, downloads a
    // newer exe next to itself, checks it, and swaps it in (a running exe can be renamed, not overwritten).
    // Local builds have version 0.0.0 and never update themselves.
    static class Updater
    {
        public const string Repo = "rkl1976/christmas";
        public const string AssetName = "JuleHygge.exe";
        public const int RestartExitCode = 3;   // tells the supervisor to start the new exe and step down

        static string Dir { get { return Path.GetDirectoryName(Program.ExePath); } }
        static string NewPath { get { return Path.Combine(Dir, "JuleHygge.new.exe"); } }

        // The version stamped by build.ps1 -Version. JULE_UPDATE_VERSION=x.y.z pretends to be another version (testing).
        public static Version Current
        {
            get
            {
                string over = Environment.GetEnvironmentVariable("JULE_UPDATE_VERSION");
                Version v;
                if (!string.IsNullOrEmpty(over) && Version.TryParse(over, out v)) return Normalize(v);
                return Normalize(Assembly.GetExecutingAssembly().GetName().Version);
            }
        }

        public static bool IsReleaseBuild { get { return Current > new Version(0, 0, 0, 0); } }

        // Only a released exe outside a package updates itself; the Store installs the packaged version's updates.
        public static bool SelfUpdates { get { return IsReleaseBuild && !Program.IsPackaged; } }

        public static string Display(Version v) { return v.Major + "." + v.Minor + "." + v.Build; }

        // "1.4" and "1.4.0.0" compare equal after this.
        static Version Normalize(Version v)
        {
            return new Version(v.Major, v.Minor, Math.Max(0, v.Build), Math.Max(0, v.Revision));
        }

        public class Result
        {
            public Version Version;     // the newer release; null when this is the latest
            public string NewExe;       // downloaded and verified exe, ready to install
            public string Error;
        }

        // Runs on a worker thread: finds the latest release and, if it is newer and has the exe, downloads and checks it.
        public static Result CheckAndDownload()
        {
            var res = new Result();
            try
            {
                CleanUp();
                ServicePointManager.SecurityProtocol |= (SecurityProtocolType)3072;   // TLS 1.2 (GitHub requires it)
                using (var wc = new WebClient())
                {
                    wc.Headers[HttpRequestHeader.UserAgent] = "JuleHygge-Updater";
                    wc.Headers[HttpRequestHeader.Accept] = "application/vnd.github+json";
                    string json = wc.DownloadString("https://api.github.com/repos/" + Repo + "/releases/latest");
                    var root = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(json);

                    string tag = root.ContainsKey("tag_name") ? root["tag_name"] as string : null;
                    Version remote;
                    if (tag == null || !Version.TryParse(tag.TrimStart('v', 'V'), out remote)) { res.Error = "unreadable release tag: " + tag; return res; }
                    remote = Normalize(remote);
                    if (remote <= Current) return res;   // up to date

                    string url = null, digest = null;
                    var assets = root.ContainsKey("assets") ? root["assets"] as IEnumerable : null;
                    if (assets != null)
                        foreach (var o in assets)
                        {
                            var a = o as Dictionary<string, object>;
                            if (a == null || !AssetName.Equals(a["name"] as string, StringComparison.OrdinalIgnoreCase)) continue;
                            url = a["browser_download_url"] as string;
                            if (a.ContainsKey("digest")) digest = a["digest"] as string;
                        }
                    // The exe is attached by the workflow a few minutes after the release is published; try again later.
                    if (url == null) { ErrorLog.Debug("update: " + tag + " has no " + AssetName + " yet"); return res; }

                    res.Version = remote;
                    wc.Headers[HttpRequestHeader.Accept] = "application/octet-stream";
                    wc.DownloadFile(url, NewPath);
                    res.Error = Verify(NewPath, remote, digest);
                    if (res.Error != null) { TryDelete(NewPath); return res; }
                    res.NewExe = NewPath;
                    ErrorLog.Debug("update: downloaded " + tag);
                }
            }
            catch (Exception ex) { res.Error = ex.Message; }
            return res;
        }

        // The file must match GitHub's SHA-256 digest (when given) and be JuleHygge with exactly the release's version.
        static string Verify(string path, Version expected, string digest)
        {
            if (!string.IsNullOrEmpty(digest) && digest.StartsWith("sha256:", StringComparison.OrdinalIgnoreCase))
            {
                string hash;
                using (var sha = SHA256.Create())
                using (var fs = File.OpenRead(path))
                    hash = BitConverter.ToString(sha.ComputeHash(fs)).Replace("-", "");
                if (!hash.Equals(digest.Substring(7), StringComparison.OrdinalIgnoreCase)) return "downloaded file does not match its SHA-256 digest";
            }
            AssemblyName name;
            try { name = AssemblyName.GetAssemblyName(path); }
            catch (Exception) { return "downloaded file is not a .NET program"; }
            if (name.Name != "JuleHygge") return "downloaded file is not JuleHygge";
            if (Normalize(name.Version) != expected) return "downloaded exe has version " + name.Version + ", expected " + expected;
            return null;
        }

        // Swaps the downloaded exe in. Runs on the UI thread right before the restart.
        public static void Install(string newExe)
        {
            string exe = Program.ExePath;
            // a unique name, because the old supervisor may still be running from the previous old file
            string old = Path.Combine(Dir, "JuleHygge.old-" + DateTime.Now.Ticks + ".exe");
            File.Move(exe, old);
            try { File.Move(newExe, exe); }
            catch { File.Move(old, exe); throw; }
        }

        // Removes leftovers from earlier updates (skips files still in use).
        static void CleanUp()
        {
            TryDelete(NewPath);
            try { foreach (var f in Directory.GetFiles(Dir, "JuleHygge.old-*.exe")) TryDelete(f); }
            catch { }
        }

        static void TryDelete(string path)
        {
            try { if (File.Exists(path)) File.Delete(path); } catch { }
        }
    }
}
