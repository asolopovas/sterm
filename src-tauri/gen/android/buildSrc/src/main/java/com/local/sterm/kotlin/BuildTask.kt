import java.io.File
import javax.inject.Inject
import org.apache.tools.ant.taskdefs.condition.Os
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.logging.LogLevel
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations

abstract class BuildTask @Inject constructor(
    private val execOperations: ExecOperations,
) : DefaultTask() {
    @get:Internal
    abstract val workingDirectoryPath: Property<String>

    @get:Input
    abstract val target: Property<String>

    @get:Input
    abstract val release: Property<Boolean>

    @TaskAction
    fun assemble() {
        val executable = "npm"
        try {
            runTauriCli(executable)
        } catch (e: Exception) {
            if (Os.isFamily(Os.FAMILY_WINDOWS)) {
                val fallbacks = listOf(
                    "$executable.exe",
                    "$executable.cmd",
                    "$executable.bat",
                )

                var lastException: Exception = e
                for (fallback in fallbacks) {
                    try {
                        runTauriCli(fallback)
                        return
                    } catch (fallbackException: Exception) {
                        lastException = fallbackException
                    }
                }
                throw lastException
            } else {
                throw e
            }
        }
    }

    private fun runTauriCli(executable: String) {
        val workingDirectoryPath = workingDirectoryPath.orNull ?: throw GradleException("workingDirectoryPath cannot be null")
        val target = target.orNull ?: throw GradleException("target cannot be null")
        val release = release.orNull ?: throw GradleException("release cannot be null")
        val args = mutableListOf("run", "--", "tauri", "android", "android-studio-script")

        if (logger.isEnabled(LogLevel.DEBUG)) {
            args.add("-vv")
        } else if (logger.isEnabled(LogLevel.INFO)) {
            args.add("-v")
        }
        if (release) {
            args.add("--release")
        }
        args.addAll(listOf("--target", target))

        execOperations.exec {
            workingDir = File(workingDirectoryPath)
            commandLine(executable, *args.toTypedArray())
        }.assertNormalExitValue()
    }
}
